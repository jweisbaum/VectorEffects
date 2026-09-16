/**
 * Settings → MCP service (spec 8.8): the switch, the port, and a client
 * configuration with the token filled in.
 */
import { useCallback, useEffect, useState } from "react";

import type { McpStatus } from "../generated/McpStatus";
import NumberField from "../NumberField";
import { api } from "../ipc";

/** The Claude Code command and a generic HTTP client entry. */
export function clientSnippets(status: McpStatus): { claudeCode: string; json: string; bridge: string } {
  const url = `http://127.0.0.1:${status.port}/mcp`;
  const auth = `Bearer ${status.token}`;
  return {
    claudeCode: `claude mcp add --transport http vectoreffects ${url} --header "Authorization: ${auth}"`,
    json: JSON.stringify({ mcpServers: { vectoreffects: { type: "http", url, headers: { Authorization: auth } } } }, null, 2),
    bridge: `npx -y mcp-remote ${url} --header "Authorization:${auth}"`,
  };
}

export default function McpSection({ onError }: { onError: (err: unknown) => void }) {
  const [status, setStatus] = useState<McpStatus | null>(null);
  const [copied, setCopied] = useState<string | null>(null);

  useEffect(() => {
    void api.mcpStatus().then(setStatus).catch(onError);
  }, [onError]);

  const apply = useCallback(
    (promise: Promise<McpStatus>) => {
      void promise.then(setStatus).catch(onError);
    },
    [onError],
  );

  const copy = useCallback((label: string, text: string) => {
    void navigator.clipboard
      ?.writeText(text)
      .then(() => setCopied(label))
      .catch(() => setCopied(null));
  }, []);

  if (status === null)
    return (
      <section>
        <h3>MCP service</h3>
        <span>…</span>
      </section>
    );
  const snippets = clientSnippets(status);
  const url = `http://127.0.0.1:${status.port}/mcp`;

  return (
    <section>
      <h3>MCP service</h3>
      <p className="settings-note">
        Lets an AI client on this computer drive the application: open and edit projects, animate, import, export and take
        pictures of the map. Nothing outside this machine can reach it, and nothing can reach it while it is off.
      </p>
      <label className="settings-field">
        <input
          type="checkbox"
          checked={status.enabled}
          onChange={(event) => apply(api.setMcp(event.target.checked, status.port))}
        />
        Enable the MCP service on this computer
      </label>
      <label className="settings-field">
        Port
        <NumberField
          value={status.port}
          min={1}
          max={65535}
          step={1}
          commitWhileTyping={false}
          onCommit={(port) => apply(api.setMcp(status.enabled, Math.round(port)))}
        />
      </label>
      {status.bind_error && <p className="settings-error">{status.bind_error}</p>}
      {status.enabled && (
        <>
          <div className="settings-field">
            <span>URL</span>
            <code>{url}</code>
          </div>
          <div className="settings-field">
            <span>Token</span>
            <code>{status.token}</code>
            <button onClick={() => apply(api.rotateMcpToken())}>Rotate token</button>
          </div>
          <div className="settings-field">
            <span>
              {status.sessions === 0
                ? "No client connected"
                : `${status.sessions} client${status.sessions === 1 ? "" : "s"} connected${status.last_tool ? `, last: ${status.last_tool}` : ""}`}
            </span>
          </div>
          <details open>
            <summary>Client configuration</summary>
            <pre className="settings-snippet">{snippets.claudeCode}</pre>
            <button onClick={() => copy("claude", snippets.claudeCode)}>Copy Claude Code command</button>
            <pre className="settings-snippet">{snippets.json}</pre>
            <button onClick={() => copy("json", snippets.json)}>Copy HTTP client JSON</button>
            <pre className="settings-snippet">{snippets.bridge}</pre>
            <button onClick={() => copy("bridge", snippets.bridge)}>Copy stdio bridge command</button>
            {copied && <span className="settings-note">Copied.</span>}
          </details>
        </>
      )}
    </section>
  );
}

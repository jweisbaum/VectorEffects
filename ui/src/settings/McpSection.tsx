/**
 * Settings → MCP service (spec 8.8): the switch, the port, a button that
 * writes the service into Claude Code's or Codex's own configuration, and
 * the same configuration as text for every other client.
 */
import { useCallback, useEffect, useRef, useState } from "react";

import type { McpClient } from "../generated/McpClient";
import type { McpStatus } from "../generated/McpStatus";
import NumberField from "../NumberField";
import { api } from "../ipc";

/** The Claude Code command and a generic HTTP client entry. */
export function clientSnippets(status: McpStatus): { claudeCode: string; json: string; bridge: string } {
  const url = `http://127.0.0.1:${status.port}/mcp`;
  const auth = `Bearer ${status.token}`;
  return {
    claudeCode: `claude mcp add --scope user --transport http vectoreffects ${url} --header "Authorization: ${auth}"`,
    json: JSON.stringify({ mcpServers: { vectoreffects: { type: "http", url, headers: { Authorization: auth } } } }, null, 2),
    bridge: `npx -y mcp-remote ${url} --header "Authorization:${auth}"`,
  };
}

const CLIENTS: readonly { client: McpClient; name: string }[] = [
  { client: "claude_code", name: "Claude Code" },
  { client: "codex", name: "Codex" },
];

/**
 * What a client was last given, so the button can say when that has gone
 * stale: a rotated token or a changed port leaves the client holding a
 * configuration the listener no longer answers.
 */
function registrationKey(status: McpStatus): string {
  return `${status.port} ${status.token}`;
}

/** The button's words: not yet added, added as it stands, or added and since changed. */
export function registerLabel(name: string, given: string | undefined, status: McpStatus): string {
  if (given === undefined) return `Add to ${name}`;
  return given === registrationKey(status) ? `Added to ${name}` : `Update in ${name}`;
}

export default function McpSection({ onError }: { onError: (err: unknown) => void }) {
  const [status, setStatus] = useState<McpStatus | null>(null);
  const [copied, setCopied] = useState<string | null>(null);
  // Remembered for as long as the dialog is open and no longer. The clients'
  // files are theirs and are not read back to find out: a reopened dialog
  // offers "Add" again, and adding twice is harmless.
  const [given, setGiven] = useState<Partial<Record<McpClient, string>>>({});
  const [adding, setAdding] = useState<McpClient | null>(null);

  // `onError` is `SettingsDialog`'s inline `report`, a new function identity
  // on every one of the dialog's re-renders (a shortcut rebind, a macro
  // deletion, a gradient load — nothing to do with this section). Reading it
  // through a ref, updated every render but never a dependency, keeps the
  // fetch-once effect below from re-running on someone else's state change.
  const onErrorRef = useRef(onError);
  onErrorRef.current = onError;

  useEffect(() => {
    void api
      .mcpStatus()
      .then(setStatus)
      .catch((err) => onErrorRef.current(err));
  }, []);

  const apply = useCallback((promise: Promise<McpStatus>) => {
    void promise.then(setStatus).catch((err) => onErrorRef.current(err));
  }, []);

  const copy = useCallback((label: string, text: string) => {
    void navigator.clipboard
      ?.writeText(text)
      .then(() => setCopied(label))
      .catch(() => setCopied(null));
  }, []);

  const register = useCallback((client: McpClient, current: McpStatus) => {
    setAdding(client);
    void api
      .registerMcpClient(client)
      .then(() => setGiven((held) => ({ ...held, [client]: registrationKey(current) })))
      .catch((err) => onErrorRef.current(err))
      .finally(() => setAdding(null));
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
      <label className="settings-field settings-check">
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
          <div className="settings-field">
            {CLIENTS.map(({ client, name }) => (
              <button key={client} disabled={adding !== null} onClick={() => register(client, status)}>
                {registerLabel(name, given[client], status)}
              </button>
            ))}
          </div>
          {CLIENTS.some(({ client }) => given[client] === registrationKey(status)) && (
            <p className="settings-note">Added. A session that is already running picks it up when it is restarted.</p>
          )}
          <details>
            <summary>Configuration for other clients</summary>
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

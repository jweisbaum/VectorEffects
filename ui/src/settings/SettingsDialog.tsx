/**
 * The Settings dialog (spec.md 8.6, M15).
 *
 * Three sections: Shortcuts, Display and Macros. The bindings it edits are the
 * same table every handler reads, so a rebind here changes the key everywhere
 * at once — including the tooltips, which is the half of a rebind that is
 * usually forgotten.
 *
 * A collision is refused by the **backend**, which is the only place that can
 * see every binding at once; this shows what it said rather than deciding for
 * itself.
 */

import { useState } from "react";

import type { AppSettings } from "../generated/AppSettings";
import type { ProjectSummary } from "../generated/ProjectSummary";
import type { Shortcut } from "../generated/Shortcut";
import NumberField from "../NumberField";
import { api } from "../ipc";
import { chordLabel } from "./bindings";

/** The rows the Shortcuts section lists, in order, with their labels. */
const ACTIONS: ReadonlyArray<{ action: Shortcut["action"]; tool: string; label: string }> = [
  { action: "play_pause", tool: "", label: "Play / pause" },
  { action: "step_back", tool: "", label: "Step back" },
  { action: "step_forward", tool: "", label: "Step forward" },
  { action: "pan_left", tool: "", label: "Pan left" },
  { action: "pan_right", tool: "", label: "Pan right" },
  { action: "pan_up", tool: "", label: "Pan up" },
  { action: "pan_down", tool: "", label: "Pan down" },
  { action: "zoom_in", tool: "", label: "Zoom in" },
  { action: "zoom_out", tool: "", label: "Zoom out" },
  { action: "tool", tool: "hand", label: "Hand tool" },
  { action: "tool", tool: "select", label: "Select tool" },
  { action: "tool", tool: "fill", label: "Fill tool" },
  { action: "tool", tool: "brush", label: "Brush" },
  { action: "tool", tool: "circle", label: "Circle" },
  { action: "tool", tool: "shape_fill", label: "Shape fill" },
  { action: "tool", tool: "mask", label: "Mask" },
  { action: "tool", tool: "clone_stamp", label: "Clone stamp" },
  { action: "tool", tool: "curve", label: "Curve" },
  { action: "tool", tool: "intensity", label: "Intensify / reduce" },
  { action: "tool", tool: "divergence", label: "Diverge / converge" },
  { action: "tool", tool: "turn", label: "Rotate flow" },
  { action: "tool", tool: "warp", label: "Warp / liquify" },
];

export default function SettingsDialog({
  settings,
  project,
  onSettings,
  onProject,
  onClose,
}: {
  settings: AppSettings;
  project: ProjectSummary | null;
  onSettings: (settings: AppSettings) => void;
  onProject: (project: ProjectSummary) => void;
  onClose: () => void;
}) {
  const [error, setError] = useState<string | null>(null);
  /** The row waiting for a key press, if any. */
  const [capturing, setCapturing] = useState<string | null>(null);

  const report = (err: unknown) => setError(String(err));

  const rebind = (row: (typeof ACTIONS)[number], event: React.KeyboardEvent) => {
    event.preventDefault();
    event.stopPropagation();
    setCapturing(null);
    // A modifier on its own is not a binding; wait for the real key.
    if (["Shift", "Meta", "Control", "Alt"].includes(event.key)) return;
    setError(null);
    void api
      .setShortcut({
        action: row.action,
        tool: row.tool,
        key: event.key.toLowerCase(),
        shift: event.shiftKey,
      })
      .then(onSettings)
      .catch(report);
  };

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div
        className="modal settings"
        role="dialog"
        aria-label="Settings"
        onClick={(event) => event.stopPropagation()}
      >
        <h2>Settings</h2>
        {error !== null && <p className="modal-error">{error}</p>}

        <section>
          <h3>Shortcuts</h3>
          <p className="muted">
            Click a key and press the one you want. A key that is already taken, or one
            the window owns, is refused.
          </p>
          <div className="shortcut-rows">
            {ACTIONS.map((row) => {
              const id = `${row.action}:${row.tool}`;
              const binding =
                settings.shortcuts.find(
                  (s) => s.action === row.action && s.tool === row.tool,
                ) ?? null;
              return (
                <div className="shortcut-row" key={id}>
                  <span>{row.label}</span>
                  <button
                    className={capturing === id ? "chord capturing" : "chord"}
                    onClick={() => setCapturing(capturing === id ? null : id)}
                    onKeyDown={(event) => {
                      if (capturing === id) rebind(row, event);
                    }}
                    title="Click, then press a key"
                  >
                    {capturing === id ? "press a key…" : chordLabel(binding)}
                  </button>
                </div>
              );
            })}
          </div>
          <button
            onClick={() => {
              setError(null);
              void api.resetShortcuts().then(onSettings).catch(report);
            }}
          >
            Reset to defaults
          </button>
        </section>

        <section>
          <h3>Display</h3>
          {/*
            The scale is the *project's* (spec.md 5.3): two people opening one
            file should see the same map. What lives in the preferences is the
            default a new project gets.
          */}
          {project !== null && (
            <label className="settings-field">
              This project&rsquo;s colour scale, in knots
              <NumberField
                value={project.colour_scale_knots}
                min={1}
                max={400}
                step={1}
                commitWhileTyping={false}
                onCommit={(value) => {
                  setError(null);
                  void api.setColourScale(value).then(onProject).catch(report);
                }}
              />
            </label>
          )}
          <label className="settings-field">
            Default for new wind projects
            <NumberField
              value={settings.default_wind_scale_knots}
              min={1}
              max={400}
              step={1}
              commitWhileTyping={false}
              onCommit={(value) => {
                void api
                  .setDefaultScales(value, settings.default_current_scale_knots)
                  .then(onSettings)
                  .catch(report);
              }}
            />
          </label>
          <label className="settings-field">
            Default for new current projects
            <NumberField
              value={settings.default_current_scale_knots}
              min={1}
              max={400}
              step={1}
              commitWhileTyping={false}
              onCommit={(value) => {
                void api
                  .setDefaultScales(settings.default_wind_scale_knots, value)
                  .then(onSettings)
                  .catch(report);
              }}
            />
          </label>
        </section>

        <section>
          <h3>Macros</h3>
          <label className="settings-field">
            Library directory
            <input
              type="text"
              value={settings.macro_directory}
              placeholder="(the default, under the app data directory)"
              onChange={(event) => {
                void api.setMacroDirectory(event.target.value).then(onSettings).catch(report);
              }}
            />
          </label>
        </section>

        <div className="modal-actions">
          <button onClick={onClose}>Close</button>
        </div>
      </div>
    </div>
  );
}

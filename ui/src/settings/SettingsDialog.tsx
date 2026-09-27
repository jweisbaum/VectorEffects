/**
 * The Settings dialog (spec.md 8.6, M15).
 *
 * The bindings it edits are the same table every handler reads, so a rebind here changes the key everywhere
 * at once — including the tooltips, which is the half of a rebind that is
 * usually forgotten.
 *
 * A collision is refused by the **backend**, which is the only place that can
 * see every binding at once; this shows what it said rather than deciding for
 * itself.
 */

import { useUnits } from "./units";

import { useEffect, useState } from "react";

import { msg, useT } from "../i18n";
import LanguagePicker from "../i18n/LanguagePicker";
import { onReveal } from "../help/highlight";
import { rich } from "./rich";

import type { AppSettings } from "../generated/AppSettings";
import type { AutosaveMode } from "../generated/AutosaveMode";
import type { MacroLibrary } from "../generated/MacroLibrary";
import type { ChartStatus } from "../generated/ChartStatus";
import { pickChartDirectory } from "../project/dialogs";
import type { ProjectSummary } from "../generated/ProjectSummary";
import type { Shortcut } from "../generated/Shortcut";
import NumberField from "../NumberField";
import { api } from "../ipc";
import { chordLabel } from "./bindings";
import type { GradientView } from "../generated/GradientView";
import { knownGradients, loadGradients } from "../gradients";
import GradientPicker from "./GradientPicker";
import GlyphSettings from "./GlyphSettings";
import McpSection from "./McpSection";
import ThemePicker from "./ThemePicker";
import ThemeEditor from "./ThemeEditor";
import type { CustomTheme } from "../generated/CustomTheme";
import { DEFAULT_THEME } from "./themes";

/** The rows the Shortcuts section lists, in order, with their labels. */
const ACTIONS: ReadonlyArray<{ action: Shortcut["action"]; tool: string; label: string }> = [
  { action: "play_pause", tool: "", label: msg("Play / pause") },
  { action: "step_back", tool: "", label: msg("Step back") },
  { action: "step_forward", tool: "", label: msg("Step forward") },
  { action: "pan_left", tool: "", label: msg("Pan left") },
  { action: "pan_right", tool: "", label: msg("Pan right") },
  { action: "pan_up", tool: "", label: msg("Pan up") },
  { action: "pan_down", tool: "", label: msg("Pan down") },
  { action: "zoom_in", tool: "", label: msg("Zoom in") },
  { action: "zoom_out", tool: "", label: msg("Zoom out") },
  { action: "nudge_left", tool: "", label: msg("Nudge selection left") },
  { action: "nudge_right", tool: "", label: msg("Nudge selection right") },
  { action: "nudge_up", tool: "", label: msg("Nudge selection up") },
  { action: "nudge_down", tool: "", label: msg("Nudge selection down") },
  { action: "tool", tool: "hand", label: msg("Hand tool") },
  { action: "tool", tool: "select", label: msg("Select tool") },
  { action: "tool", tool: "brush", label: msg("Brush") },
  { action: "tool", tool: "circle", label: msg("Circle") },
  { action: "tool", tool: "shape_fill", label: msg("Shape fill") },
  { action: "tool", tool: "mask", label: msg("Mask") },
  { action: "tool", tool: "clone_stamp", label: msg("Clone stamp") },
  { action: "tool", tool: "curve", label: msg("Curve") },
  { action: "tool", tool: "intensity", label: msg("Intensify / reduce") },
  { action: "tool", tool: "divergence", label: msg("Diverge / converge") },
  { action: "tool", tool: "turn", label: msg("Rotate flow") },
  { action: "tool", tool: "warp", label: msg("Warp") },
  { action: "tool", tool: "liquify", label: msg("Displace") },
  { action: "tool", tool: "measure", label: msg("Measure") },
  { action: "tool", tool: "capture", label: msg("Capture a macro") },
  { action: "tool", tool: "insert", label: msg("Insert a macro") },
];

export default function SettingsDialog({
  settings,
  project,
  onSettings,
  onProject,
  onLibrary,
  onClose,
}: {
  settings: AppSettings;
  project: ProjectSummary | null;
  onSettings: (settings: AppSettings) => void;
  onProject: (project: ProjectSummary) => void;
  /**
   * The macro library changed (M35).
   *
   * The insert tool reads the library when it is picked up and keeps what it
   * read, so a library emptied from here left the stamp tool offering macros
   * that are no longer on disk.
   */
  onLibrary: () => void;
  onClose: () => void;
}) {
  const t = useT();
  const units = useUnits();
  const [error, setError] = useState<string | null>(null);
  const [customDraft, setCustomDraft] = useState<CustomTheme | null>(null);
  /** The row waiting for a key press, if any. */
  const [capturing, setCapturing] = useState<string | null>(null);
  const [library, setLibrary] = useState<MacroLibrary | null>(null);
  const [chartStatus, setChartStatus] = useState<ChartStatus | null>(null);

  useEffect(() => {
    let live = true;
    void api
      .chartStatus()
      .then((status) => { if (live) setChartStatus(status); })
      .catch(() => { if (live) setChartStatus(null); });
    return () => { live = false; };
  }, []);
  /** The gradients on offer (M42), from the backend's own table. */
  const [gradients, setGradients] = useState<readonly GradientView[]>(knownGradients);
  useEffect(() => {
    let dropped = false;
    void loadGradients()
      .then((list) => {
        if (!dropped) setGradients(list);
      })
      .catch(() => undefined);
    return () => {
      dropped = true;
    };
  }, []);
  /** Whether the "delete every macro" confirmation is up (M35). */
  const [confirmClear, setConfirmClear] = useState(false);
  useEffect(() => {
    void api.macroLibrary().then(setLibrary).catch(() => undefined);
  }, []);

  const report = (err: unknown) => setError(String(err));

  /**
   * The Help search's second reveal step (spec.md 5.7): `settings:` opened the
   * dialog, and `settings:<section>` brings that section into view. Each is
   * written out, not looped over, so the registry's test can read them.
   */
  useEffect(() => {
    const show = (step: string) => {
      document.querySelector(`[data-feature="${step}"]`)?.scrollIntoView?.({ block: "start" });
    };
    const offs = [
      onReveal("settings:appearance", show),
      onReveal("settings:glyphs", show),
      onReveal("settings:shortcuts", show),
      onReveal("settings:autosave", show),
      onReveal("settings:units", show),
      onReveal("settings:display", show),
      onReveal("settings:charts", show),
      onReveal("settings:macros", show),
      onReveal("settings:mcp", show),
    ];
    return () => { for (const off of offs) off(); };
  }, []);

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
        alt: event.altKey,
        accel: event.metaKey || event.ctrlKey,
      })
      .then(onSettings)
      .catch(report);
  };

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div
        className="modal settings"
        role="dialog"
        aria-label={t("Settings")}
        onClick={(event) => event.stopPropagation()}
      >
        <h2>{t("Settings")}</h2>
        {error !== null && <p className="modal-error">{error}</p>}

        <section data-feature="settings:appearance">
          <h3>{t("Appearance")}</h3>
          <ThemePicker value={settings.theme ?? DEFAULT_THEME} custom={settings.custom_theme} onChoose={id => {
            setError(null);
            setCustomDraft(null);
            void api.setTheme(id).then(onSettings).catch(report);
          }} />
          <label className="settings-field" data-feature="settings:language">
            {t("Language")}
            <LanguagePicker onSettings={onSettings} />
          </label>
          <p className="muted">{t("Applies throughout the app, across all projects.")}</p>
          {customDraft ? <ThemeEditor initial={customDraft} onCancel={() => setCustomDraft(null)} onSave={async custom => {
            setError(null);
            const saved = await api.setCustomTheme(custom);
            onSettings(saved);
            setCustomDraft(null);
          }} /> : <button type="button" className="theme-customize" data-feature="settings:custom-theme" onClick={() => {
            setCustomDraft(settings.theme === "custom" && settings.custom_theme
              ? settings.custom_theme : { base: settings.theme ?? DEFAULT_THEME, colours: {} });
          }}>{settings.theme === "custom" ? t("Edit custom theme…") : t("Customize…")}</button>}
        </section>

        <GlyphSettings settings={settings} onSettings={value => { setError(null); onSettings(value); }} onError={report} />

        <section data-feature="settings:shortcuts">
          <h3>{t("Shortcuts")}</h3>
          <p className="muted">
            {t("Click a key and press the one you want, with Shift or Alt held if you like. A key that is already taken, or one the window owns, is refused.")}
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
                  <span>{t(row.label)}</span>
                  <button
                    className={capturing === id ? "chord capturing" : "chord"}
                    onClick={() => setCapturing(capturing === id ? null : id)}
                    onKeyDown={(event) => {
                      if (capturing === id) rebind(row, event);
                    }}
                    title={t("Click, then press a key")}
                  >
                    {capturing === id ? t("press a key…") : chordLabel(binding)}
                  </button>
                </div>
              );
            })}
          </div>
          <button
            data-feature="settings:shortcuts-reset"
            onClick={() => {
              setError(null);
              void api.resetShortcuts().then(onSettings).catch(report);
            }}
          >
            {t("Reset to defaults")}
          </button>
        </section>

        <section data-feature="settings:autosave">
          <h3>{t("Autosave")}</h3>
          <label className="settings-field">
            {t("While you work, unsaved changes are")}
            <select
              value={settings.autosave}
              onChange={(event) => {
                setError(null);
                void api
                  .setAutosaveMode(event.target.value as AutosaveMode)
                  .then(onSettings)
                  .catch(report);
              }}
              title={t("Every minute, or every fifty edits, whichever comes first")}
            >
              <option value="recovery">{t("kept as a recovery snapshot, offered back after a crash")}</option>
              <option value="save">{t("saved into the project file itself")}</option>
              <option value="off">{t("left until you save")}</option>
            </select>
          </label>
        </section>

        <section data-feature="settings:units">
          <h3>{t("Units")}</h3>
          <label className="settings-field" data-feature="settings:distance-unit">
            {t("Distance")}
            <select value={settings.distance_unit} onChange={(e) => {
              void api.setDisplayUnits(e.target.value as AppSettings["distance_unit"], settings.speed_unit).then(onSettings).catch(report);
            }}>
              <option value="km">{t("km — kilometres")}</option>
              <option value="nm">{t("nm — nautical miles")}</option>
            </select>
          </label>
          <label className="settings-field" data-feature="settings:speed-unit">
            {t("Speed")}
            <select value={settings.speed_unit} onChange={(e) => {
              void api.setDisplayUnits(settings.distance_unit, e.target.value as AppSettings["speed_unit"]).then(onSettings).catch(report);
            }}>
              <option value="kt">{t("kt — knots")}</option>
              <option value="mph">{t("mph — miles per hour")}</option>
              <option value="kmh">{t("km/h — kilometres per hour")}</option>
            </select>
          </label>
        </section>

        <section data-feature="settings:display">
          <h3>{t("Display")}</h3>
          {/*
            The scale is the *project's* (spec.md 5.3): two people opening one
            file should see the same map. What lives in the preferences is the
            default a new project gets.
          */}
          {project !== null && (
            <label className="settings-field" data-feature="settings:colour-scale">
              {t("This project’s colour scale for wind, in {unit}", { unit: units.speedUnit })}
              <NumberField
                value={units.speedFromKnots(project.wind_scale_knots)}
                min={units.speedFromKnots(1)}
                max={units.speedFromKnots(400)}
                step={1}
                commitWhileTyping={false}
                onCommit={(value) => {
                  setError(null);
                  void api.setColourScale("wind", units.speedToKnots(value)).then(onProject).catch(report);
                }}
              />
            </label>
          )}
          {project !== null && (
            <label className="settings-field">
              {t("This project’s colour scale for currents, in {unit}", { unit: units.speedUnit })}
              <NumberField
                value={units.speedFromKnots(project.current_scale_knots)}
                min={units.speedFromKnots(1)}
                max={units.speedFromKnots(400)}
                step={1}
                commitWhileTyping={false}
                onCommit={(value) => {
                  setError(null);
                  void api.setColourScale("current", units.speedToKnots(value)).then(onProject).catch(report);
                }}
              />
            </label>
          )}
          {/*
            The gradient is the project's too, and for the same reason: it is
            what the map is painted with, so two people opening one file
            should see the same colours (M42). The picker shows the colours
            rather than only naming them, since a column of names asks the
            reader to remember what "Haxby" looks like — which is what they
            opened the settings to find out.
          */}
          {project !== null &&
            (["wind", "current"] as const).map((kind) => {
              const chosen = project[`${kind}_gradient`];
              // A project written by a later version may name a gradient this
              // build has never heard of. It is drawn with the default, and
              // listed with no colours and no way to pick it, so the control
              // says what the file says rather than reading as something else.
              const known = gradients.map((entry) => ({ ...entry, known: true }));
              const rows = gradients.some((entry) => entry.id === chosen)
                ? known
                : [
                    ...known,
                    {
                      id: chosen,
                      label: t("{name} (not in this version)", { name: chosen }),
                      note: t("Drawn with the default until this project is opened by the version that has it."),
                      stops: [],
                      known: false,
                    },
                  ];
              return (
                <div key={kind} data-feature={`settings:gradient-${kind}`}>
                  <GradientPicker
                    label={
                      kind === "wind"
                        ? t("This project’s colour gradient for wind")
                        : t("This project’s colour gradient for currents")
                    }
                    value={chosen}
                    gradients={rows}
                    onChoose={(id) => {
                      setError(null);
                      void api.setColourGradient(kind, id).then(onProject).catch(report);
                    }}
                  />
                </div>
              );
            })}
          <label className="settings-field" data-feature="settings:default-scales">
            {t("Default for new wind projects, in {unit}", { unit: units.speedUnit })}
            <NumberField
              value={units.speedFromKnots(settings.default_wind_scale_knots)}
              min={units.speedFromKnots(1)}
              max={units.speedFromKnots(400)}
              step={1}
              commitWhileTyping={false}
              onCommit={(value) => {
                void api
                  .setDefaultScales(units.speedToKnots(value), settings.default_current_scale_knots)
                  .then(onSettings)
                  .catch(report);
              }}
            />
          </label>
          <label className="settings-field">
            {t("Default for new current projects, in {unit}", { unit: units.speedUnit })}
            <NumberField
              value={units.speedFromKnots(settings.default_current_scale_knots)}
              min={units.speedFromKnots(1)}
              max={units.speedFromKnots(400)}
              step={1}
              commitWhileTyping={false}
              onCommit={(value) => {
                void api
                  .setDefaultScales(settings.default_wind_scale_knots, units.speedToKnots(value))
                  .then(onSettings)
                  .catch(report);
              }}
            />
          </label>
        </section>

        <section data-feature="settings:charts">
          <h3>{t("Charts")}</h3>
          <p className="muted">
            {rich(t("Electronic navigational charts (S-57). Choose the directory an exchange set was unpacked into — the one holding the cell folders, usually beside a {file}. Turn them on with {charts} in the view controls. Charts are drawn under the field and are never part of a project."), {
              // i18n-ignore: a file name
              file: <code>CATALOG.031</code>,
              charts: <strong>{t("Charts")}</strong>,
            })}
          </p>
          <div className="settings-field">
            <span>
              {chartStatus === null
                ? "…"
                : chartStatus.error
                  ? chartStatus.error
                  : chartStatus.directory
                    ? chartStatus.cells === 1
                      ? t("1 cell in {directory}", { directory: chartStatus.directory })
                      : t("{count} cells in {directory}", { count: chartStatus.cells.toLocaleString(), directory: chartStatus.directory })
                    : t("No chart directory chosen.")}
            </span>
            <span className="settings-actions">
              <button
                data-feature="settings:chart-directory"
                onClick={() => {
                  void (async () => {
                    const directory = await pickChartDirectory();
                    if (directory === null) return;
                    await api
                      .setChartDirectory(directory)
                      .then((status) => {
                        setChartStatus(status);
                        void api.appSettings().then(onSettings);
                      })
                      .catch(report);
                  })();
                }}
              >
                {t("Choose…")}
              </button>
              <button
                disabled={!chartStatus?.directory}
                onClick={() => {
                  void api
                    .setChartDirectory("")
                    .then((status) => {
                      setChartStatus(status);
                      void api.appSettings().then(onSettings);
                    })
                    .catch(report);
                }}
              >
                {t("Clear")}
              </button>
            </span>
          </div>
        </section>

        <section data-feature="settings:macros">
          <h3>{t("Macros")}</h3>
          <label className="settings-field" data-feature="settings:macro-directory">
            {t("Library directory")}
            <input
              type="text"
              value={settings.macro_directory}
              placeholder={t("(the default, under the app data directory)")}
              onChange={(event) => {
                void api.setMacroDirectory(event.target.value).then(onSettings).catch(report);
              }}
            />
          </label>
          {/*
            Safe to press: a project that used a macro carries its own copy of
            the frames, so clearing the library breaks nothing already
            inserted (spec.md 8.7, D52). The button says so rather than asking
            the user to remember it.
          */}
          <div className="settings-field">
            <span>
              {library === null
                ? "…"
                : library.entries.length === 1
                  ? t("1 macro, {size}", { size: formatBytes(library.total_bytes) })
                  : t("{count} macros, {size}", { count: library.entries.length, size: formatBytes(library.total_bytes) })}
            </span>
            <button
              data-feature="settings:delete-macros"
              disabled={library === null || library.entries.length === 0}
              title={t("Projects that already use a macro keep their own copy of its frames, so this breaks nothing")}
              onClick={() => setConfirmClear(true)}
            >
              {t("Delete all macros")}
            </button>
          </div>
        </section>

        <McpSection onError={report} />

        <div className="modal-actions">
          <button onClick={onClose}>{t("Close")}</button>
        </div>
      </div>

      {/*
        Deleting the library is not undoable and reaches outside the project,
        so it is asked for (M35) — with the number and the size, and with the
        one thing that makes it safe: a project that used a macro carries its
        own copy of the frames (D52).
      */}
      {confirmClear && library !== null && (
        <div className="modal-backdrop" onClick={() => setConfirmClear(false)}>
          <div
            className="modal modal-narrow"
            role="dialog"
            aria-label={t("Delete all macros")}
            onClick={(event) => event.stopPropagation()}
          >
            <h2>
              {library.entries.length === 1
                ? t("Delete 1 macro?")
                : t("Delete {count} macros?", { count: library.entries.length })}
            </h2>
            <p>
              {t("This removes {size} from the macro library on disk. It cannot be undone.", { size: formatBytes(library.total_bytes) })}
            </p>
            <p className="muted">
              {t("Projects that already use a macro keep their own copy of its frames, so nothing you have placed will change.")}
            </p>
            <div className="modal-actions">
              <button onClick={() => setConfirmClear(false)}>{t("Cancel")}</button>
              <button
                className="danger"
                onClick={() => {
                  setConfirmClear(false);
                  setError(null);
                  void api
                    .deleteMacros(null)
                    .then((held) => {
                      setLibrary(held);
                      onLibrary();
                    })
                    .catch(report);
                }}
              >
                {library.entries.length === 1
                  ? t("Delete 1 macro")
                  : t("Delete {count} macros", { count: library.entries.length })}
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}

/** A byte count as the settings dialog shows it. */
function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(0)} kB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

import { UnitsProvider } from "./settings/units";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import ExportDialog from "./project/ExportDialog";
import HistoryPanel from "./panels/HistoryPanel";
import Inspector from "./panels/Inspector";
import LayerPanel from "./panels/LayerPanel";
import MapView, { type MapHandle } from "./map/MapView";
import Timeline from "./timeline/Timeline";
import NewProjectDialog from "./project/NewProjectDialog";
import StartScreen from "./project/StartScreen";
import BetaGate from "./project/BetaGate";
import Help from "./help/Help";
import UnsavedChangesDialog from "./project/UnsavedChangesDialog";
import { mayReplaceProject, type UnsavedChoice } from "./project/saveGuard";
import { pickProjectToOpen, pickProjectToSave } from "./project/dialogs";
import type { CSSProperties } from "react";

import { stillPasteChord } from "./chords";
import type { FieldKindName } from "./kind";
import { listen } from "@tauri-apps/api/event";
import { api, HISTORY_LABEL, IpcError } from "./ipc";
import { reportError, retryError, shown, useHint } from "./hint";
import { isBusy, useBusy } from "./busy";
import type { HistoryProgress } from "./generated/HistoryProgress";
import { type PanelState, loadPanels, savePanels, togglePanel } from "./panels/layout";
import SettingsDialog from "./settings/SettingsDialog";
import type { AppInfo } from "./generated/AppInfo";
import type { AppSettings } from "./generated/AppSettings";
import type { CaptureMode } from "./generated/CaptureMode";
import type { ProjectSummary } from "./generated/ProjectSummary";
import type { TileAddress } from "./generated/TileAddress";
import type { PositionPick } from "./picking";

/**
 * Application shell.
 *
 * No project, no map: the map's timeline, direction convention and colour
 * scale all come from `ProjectSettings`, so rendering one without a project
 * would mean inventing those values (spec.md 2).
 */
export default function App() {
  return <BetaGate><Help><EditorApp /></Help></BetaGate>;
}

function EditorApp() {
  const [info, setInfo] = useState<AppInfo | null>(null);
  const [project, setProject] = useState<ProjectSummary | null>(null);
  // A failed download's saved request belongs to this opening of the project.
  useEffect(() => reportError(null), [project?.image_token]);
  const [status, setStatus] = useState<string | null>(null);
  const [exporting, setExporting] = useState(false);
  // Non-null while the New Project dialog is up; the boolean is the answer the
  // user already gave about unsaved changes, carried through to the command.
  const [creating, setCreating] = useState<{ discardUnsaved: boolean } | null>(null);
  // Set while the unsaved-changes prompt is up: holds the resolver the dialog's
  // buttons complete, which is what lets the guard read as a plain `await`.
  const [askUnsaved, setAskUnsaved] = useState<{
    name: string;
    resolve: (choice: UnsavedChoice) => void;
  } | null>(null);
  // The step and the selection live here because the map, the layer panel and
  // the inspector all need them.
  const [step, setStep] = useState(0);
  // Object ids, in the order they were selected. A list rather than one id
  // because transforms act on the whole selection about its collective
  // centroid (spec.md 8.2).
  const [selection, setSelection] = useState<number[]>([]);
  const [shapeEditing, setShapeEditing] = useState<number | null>(null);
  const exitShapeEditing = useCallback(() => setShapeEditing(null), []);
  const toggleShapeEditing = useCallback((object: number) => {
    setShapeEditing((current) => current === object ? null : object);
    setSelection([object]);
    mapRef.current?.clearRegion();
    setPicking(null);
  }, []);
  /**
   * Whether the timeline has a GRIB frame selected, in which case copy and
   * paste belong to it and not to the object clipboard (spec.md 4.8, M20).
   * A ref rather than state: the key handler reads it and nothing draws it.
   */
  const framesSelected = useRef(false);
  const onFramesSelected = useCallback((active: boolean) => {
    framesSelected.current = active;
  }, []);
  /**
   * Whether the timeline has keyframes selected, in which case `Delete`
   * belongs to them and not to the objects (spec.md 9.3). A ref for the same
   * reason `framesSelected` is.
   */
  const keysSelected = useRef(false);
  const onKeysSelected = useCallback((active: boolean) => {
    keysSelected.current = active;
  }, []);
  /**
   * The application's settings, loaded once. Every key handler reads the
   * bindings from here, so a rebind changes the key everywhere (M15).
   */
  const [settings, setSettings] = useState<AppSettings | null>(null);
  const [showSettings, setShowSettings] = useState(false);
  /**
   * The macro capture in progress, reported by the map (spec.md 8.7, M16).
   *
   * While one runs every document write is refused, so the edit shortcuts
   * stand down here rather than firing and being refused with an error.
   */
  const [recording, setRecording] = useState<CaptureMode | null>(null);
  useEffect(() => {
    // A settings file that will not load costs the preferences and not the
    // launch: the backend already falls back to the defaults, so a failure
    // here is a failure to *ask*, and the app runs with none rather than
    // refusing to start.
    void api.appSettings().then(setSettings).catch(() => undefined);
  }, []);
  /**
   * Selects objects — and drops the map's region, because the two are
   * mutually exclusive (spec.md 8.2, M23): a region is a way of pointing at
   * ground, and an object selection a way of pointing at things, and a
   * gesture that means one cannot leave the other standing. The map does the
   * converse when a region is drawn.
   */
  const selectObjects = useCallback((ids: number[]) => {
    if (ids.length > 0) mapRef.current?.clearRegion();
    setSelection(ids);
    setShapeEditing((current) => current !== null && ids.length === 1 && ids[0] === current ? current : null);
  }, []);
  // Which layer receives new objects, and what a plain marquee is scoped to
  // (spec.md 6.1, 8.2). Null means the top of the stack.
  const [activeLayer, setActiveLayer] = useState<number | null>(null);
  /**
   * The kind of field the active layer paints (M29, M30): what the eyedropper
   * samples and a capture takes. The map shows every kind the project holds.
   */
  const [activeKind, setActiveKind] = useState<FieldKindName>("wind");
  /**
   * Bumped whenever the macro library changes from outside the map (M35).
   *
   * The insert tool reads the library when it is picked up and keeps what it
   * read; emptying the library from the settings left it offering macros
   * that are no longer there. A counter rather than the library itself: the
   * map is the one that knows how to read it, and only needs telling that
   * what it has is stale.
   */
  const [libraryRevision, setLibraryRevision] = useState(0);
  // Armed by the inspector, answered by the map: the next map click places this
  // property of this object.
  const [picking, setPicking] = useState<PositionPick | null>(null);
  /**
   * Which panels are open (M25). A viewer's convenience, remembered in
   * `localStorage`, never a project's fact.
   */
  const [panels, setPanels] = useState<PanelState>(loadPanels);
  const toggle = useCallback((panel: keyof PanelState) => {
    setPanels((current) => {
      const next = togglePanel(current, panel);
      savePanels(next);
      return next;
    });
  }, []);
  /** The project's name being edited in the title bar, if it is (M25). */
  const [renaming, setRenaming] = useState<string | null>(null);
  /**
   * The title bar's centre slot, which the map fills with its view controls
   * through a portal (M25, D68). State rather than a ref so the map re-renders
   * once the element exists.
   */
  const [viewSlot, setViewSlot] = useState<HTMLElement | null>(null);
  // Auto-key (spec.md 9.3): while on, an edit in the inspector keys the
  // current step instead of changing the property's base.
  const [autoKey, setAutoKey] = useState(false);
  // The map's visible tiles, which the timeline renders ahead for and asks
  // readiness about (spec.md 9.5). The map reports them; nothing else knows.
  const [viewport, setViewport] = useState<TileAddress[]>([]);
  const mapRef = useRef<MapHandle | null>(null);
  // The map holds the tiles; playback asks it, not the backend, whether a step
  // can be shown (spec.md 9.4). Before the map exists there is nothing to wait for.
  const playback = useMemo(() => ({
    prepare: (request: import("./timeline/preparation").PreparationRequest) =>
      mapRef.current?.prepare(request) ?? { ready: 0, total: 1, streaming: false },
    present: (target: number) => mapRef.current?.present(target) ?? false,
  }), []);
  /** Where the map is looking, for an image that has to be placed by hand. */
  const viewBounds = useCallback(() => mapRef.current?.bounds() ?? null, []);

  // A shorter timeline cannot leave the playhead past its end.
  useEffect(() => {
    if (project && step > project.step_count - 1) setStep(Math.max(0, project.step_count - 1));
  }, [project, step]);

  // An armed pick belongs to a selected object. Leaving it armed after the
  // selection moves on would send the next click to something the panels are no
  // longer showing.
  useEffect(() => {
    if (picking !== null && !selection.includes(picking.object)) setPicking(null);
  }, [picking, selection]);

  useEffect(() => {
    void (async () => {
      const [appInfo, open] = await Promise.all([
        api.appInfo().catch(() => null),
        // A project may already be open after a hot reload of the frontend.
        api.currentProject().catch(() => null),
      ]);
      setInfo(appInfo);
      if (open) {
        setProject(open);
        return;
      }
      // The capture suite needs a project to render; create a deterministic one
      // so development captures do not depend on clicking through the dialog.
      if (appInfo?.debug_capture) {
        setProject(
          await api.newProject({
            name: "Capture",
            field_kind: "wind",
            resolution: "0.25",
            step_hours: 3,
            step_count: 24,
          }),
        );
      }
    })();
  }, []);

  // The message alone (M59). The `kind` is a discriminant for code, not a word
  // for a person — `[bad-option]` in front of a sentence reads as machine
  // trouble whatever the sentence says — so it goes in the line's tooltip,
  // where a bug report can still find it.
  const report = (err: unknown) => {
    if (err instanceof IpcError) reportError(err.message, err.kind);
    else reportError(String(err));
  };

  const flash = (message: string) => {
    setStatus(message);
    reportError(null);
    window.setTimeout(() => setStatus((current) => (current === message ? null : current)), 2500);
  };

  // Both report whether the project actually reached disk. A cancelled
  // destination dialog is not a save, and the unsaved-changes guard has to be
  // able to tell the difference before it lets an operation discard anything.
  const saveAs = useCallback(async (): Promise<boolean> => {
    if (!project) return false;
    try {
      const path = await pickProjectToSave(project.name);
      if (path === null) return false;
      const saved = await api.saveProjectAs(path);
      setProject(saved);
      flash(`Saved to ${saved.path ?? path}`);
      return true;
    } catch (err) {
      report(err);
      return false;
    }
  }, [project]);

  const save = useCallback(async (): Promise<boolean> => {
    if (!project) return false;
    // Never saved: fall through to Save As rather than failing.
    if (project.path === null) return saveAs();
    try {
      setProject(await api.saveProject());
      flash("Saved");
      return true;
    } catch (err) {
      report(err);
      return false;
    }
  }, [project, saveAs]);

  // Asks about unsaved changes, if there are any, and reports whether the
  // caller may go ahead and replace the open project.
  const mayReplace = useCallback(
    () =>
      mayReplaceProject(
        project,
        () =>
          new Promise<UnsavedChoice>((resolve) => {
            // A second prompt would strand the first one's promise unresolved.
            if (askUnsaved !== null) {
              resolve("cancel");
              return;
            }
            setAskUnsaved({ name: project?.name ?? "This project", resolve });
          }),
        save,
      ),
    [askUnsaved, project, save],
  );

  const answerUnsaved = useCallback(
    (choice: UnsavedChoice) => {
      askUnsaved?.resolve(choice);
      setAskUnsaved(null);
    },
    [askUnsaved],
  );

  const startNewProject = useCallback(async () => {
    const decision = await mayReplace();
    if (decision.proceed) setCreating({ discardUnsaved: decision.discardUnsaved });
  }, [mayReplace]);

  /**
   * Puts the project down and goes back to the start screen.
   *
   * Through the same guard every other discard uses, so unsaved work is asked
   * about rather than dropped. Without this a project could be opened and never
   * closed: the start screen, and with it the recent list, was reachable only
   * at launch.
   */
  const closeProject = useCallback(async () => {
    const decision = await mayReplace();
    if (!decision.proceed) return;
    try {
      await api.closeProject(decision.discardUnsaved);
      setSelection([]);
      setShapeEditing(null);
      setActiveLayer(null);
      setStep(0);
      reportError(null);
      setProject(null);
    } catch (err) {
      report(err);
    }
  }, [mayReplace]);

  /**
   * Opens a path and puts the application into it.
   *
   * The command is only half of opening: the rest is this state, and a caller
   * that invokes `open_project` without it leaves the backend holding a
   * project the interface never shows (M76).
   */
  const openPath = useCallback(async (path: string, discardUnsaved: boolean) => {
    const opened = await api.openProject(path, discardUnsaved);
    setSelection([]);
    setShapeEditing(null);
    setActiveLayer(null);
    setStep(0);
    reportError(null);
    setProject(opened);
    return opened;
  }, []);

  const openProject = useCallback(async () => {
    // Ask about unsaved changes before the file dialog, not after: a user who
    // has picked a file has already decided, and asking then reads as the app
    // second-guessing them. Nothing is discarded by asking — the answer travels
    // with the open call — so cancelling the file dialog costs nothing.
    const decision = await mayReplace();
    if (!decision.proceed) return;
    try {
      const path = await pickProjectToOpen();
      if (path === null) return;
      await openPath(path, decision.discardUnsaved);
    } catch (err) {
      report(err);
    }
  }, [mayReplace, openPath]);

  /**
   * Opening, reachable from a script injected into the webview (M76).
   *
   * The automation driver has no file dialog to drive, and invoking
   * `open_project` on its own moves the backend and not the interface — the
   * application sits on the start screen holding a project it will not show,
   * which reads as the open having failed. This is the same door
   * `__veCapture` is, for the same reason and under the same guard.
   *
   * **Development builds only.** `import.meta.env.DEV` is false in anything
   * `npm run build` produces, so a shipped bundle carries no such hook. It
   * discards unsaved changes without asking, which is right for a driver and
   * is exactly why it must not exist in a shipped one.
   */
  useEffect(() => {
    if (!import.meta.env.DEV) return;
    const hooks = window as unknown as {
      __veOpen?: (path: string) => Promise<string>;
    };
    hooks.__veOpen = async (path: string) => (await openPath(path, true)).name;
    return () => {
      delete hooks.__veOpen;
    };
  }, [openPath]);

  // Standard shortcuts, so saving does not require reaching for the toolbar.
  const modal = exporting || creating !== null || askUnsaved !== null || recording !== null;
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      // A dialog is a decision in progress; undoing or saving behind it would
      // change the very thing being decided about.
      if (modal) return;
      const target = event.target as HTMLElement | null;
      const typing = target !== null && /^(INPUT|TEXTAREA|SELECT)$/.test(target.tagName);
      const key = event.key.toLowerCase();

      // `Delete` removes the selection, from the map or the panel, as one
      // history entry (spec.md 8.4, M23) — unless the timeline holds keys or
      // frames, whose own `Delete` comes first, or the key is going into a
      // text field.
      if ((key === "delete" || key === "backspace") && !typing && !event.metaKey && !event.ctrlKey) {
        if (selection.length === 0 || framesSelected.current || keysSelected.current) return;
        event.preventDefault();
        void api
          .removeObjects(selection)
          .then((next) => {
            setSelection([]);
            setShapeEditing(null);
            setProject(next);
          })
          .catch(report);
        return;
      }

      const accel = event.metaKey || event.ctrlKey;
      if (!accel) return;

      if (key === "s") {
        event.preventDefault();
        void (event.shiftKey ? saveAs() : save());
      } else if (key === "n") {
        event.preventDefault();
        void startNewProject();
      } else if (key === "o") {
        event.preventDefault();
        void openProject();
      } else if (framesSelected.current && (key === "c" || key === "v")) {
        // The timeline owns copy and paste while one of an imported layer's
        // frames is selected (spec.md 4.8, M20). Nothing to do here: its own
        // handler has already acted.
      } else if (key === "c") {
        // One clipboard (spec.md 8.5): a region selected means the field
        // inside it, otherwise the selected objects. Either copy drops the
        // other, so `Cmd`-`V` can ask what is held rather than remember.
        if (mapRef.current?.copyRegion()) {
          event.preventDefault();
        } else if (selection.length > 0) {
          event.preventDefault();
          void api.copyObjects(selection, step).catch(report);
        }
      } else if (key === "x" && selection.length > 0) {
        event.preventDefault();
        void api
          .cutObjects(selection, step)
          .then((next) => {
            setSelection([]);
            setShapeEditing(null);
            setProject(next);
          })
          .catch(report);
      } else if (key === "v") {
        event.preventDefault();
        // What is held decides what is pasted. Shift pastes objects at the
        // original step numbers instead of moving the animation to the
        // current one (spec.md 8.5). A *still* paste — `Ctrl`-`Shift`-`V` on
        // a Mac, where `Ctrl` is free, `Ctrl`-`Alt`-`Shift`-`V` elsewhere,
        // where `Ctrl`-`Shift`-`V` is already the absolute one — pastes the
        // copied frame alone, with no animation at all (M27). All land in
        // the active layer (D66).
        const still = stillPasteChord(event);
        const absolute = event.shiftKey && !still;
        void api
          .clipboardKind()
          .then((kind) => {
            if (kind === "capture") mapRef.current?.pasteCapture(activeLayer, still);
            else if (kind === "objects")
              return api.pasteObjects(activeLayer, step, absolute, still).then(setProject);
            return undefined;
          })
          .catch(report);
      } else if (key === ",") {
        event.preventDefault();
        setShowSettings(true);
      } else if (key === "z") {
        event.preventDefault();
        void (event.shiftKey ? api.redo() : api.undo())
          .then(setProject)
          .catch(report);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [activeLayer, modal, openProject, save, saveAs, selection, startNewProject, step]);

  if (!project) {
    return <StartScreen onOpened={setProject} />;
  }

  return (
    <UnitsProvider settings={settings}>
    <div className="app">
      <div className="titlebar">
        <span className="brand">VectorEffects</span>
        {/*
          The name is edited where it is shown (M25): click it, type, Enter.
          A document write like a layer's rename, so it undoes.
        */}
        {renaming !== null ? (
          <input
            className="project-name"
            autoFocus
            value={renaming}
            aria-label="Project name"
            onChange={(event) => setRenaming(event.target.value)}
            onBlur={() => {
              const name = renaming.trim();
              setRenaming(null);
              if (name.length > 0 && name !== project.name) {
                void api.renameProject(name).then(setProject).catch(report);
              }
            }}
            onKeyDown={(event) => {
              if (event.key === "Enter") event.currentTarget.blur();
              if (event.key === "Escape") setRenaming(null);
            }}
          />
        ) : (
          <button
            className="project-name"
            onClick={() => setRenaming(project.name)}
            title="Click to rename the project"
          >
            {project.name}
            {project.dirty && <span className="dirty" title="Unsaved changes"> •</span>}
          </button>
        )}
        <span className="muted project-meta">
          {project.resolution_label} · {project.step_count} × {project.step_hours} h
        </span>
        <span className="spacer" />
        {/* The map's view controls and the capture tool land here (D68). */}
        <div className="titlebar-centre" ref={setViewSlot} />
        <span className="spacer" />
        <button onClick={() => void startNewProject()}>New…</button>
        <button onClick={() => void openProject()}>Open…</button>
        <button onClick={() => void save()}>Save</button>
        <button onClick={() => void saveAs()}>Save As…</button>
        {/*
          A project could be opened but never put down: the start screen — and
          with it the recent list and the templates — was reachable only at
          launch. Closing goes through the same guard every other discard does,
          so unsaved work is asked about rather than dropped.
        */}
        <button onClick={() => void closeProject()}>Close</button>
        {/*
          The shortcut alone is not a way to find something. `Cmd`-`,` is where
          every Mac application keeps its preferences and is worth binding, but
          a feature reachable only by a chord nobody was told about is a feature
          nobody opens.
        */}
        <button
          className="settings"
          onClick={() => setShowSettings(true)}
          title="Settings (Cmd+,) · shortcuts, default scales, the macro library"
          aria-label="Settings"
        >
          ⚙
        </button>
      </div>

      {/*
        The stage: the map fills it, edge to edge, and the panels sit *over*
        the map rather than beside it (M27). Opening or closing a panel then
        changes nothing about the map's size, so no tile is re-rendered and no
        frame is redrawn for it; the map's own chrome keeps clear of the
        docked panels through the inset variables below.
      */}
      <div
        className="stage"
        style={
          {
            "--dock-left": panels.left ? "310px" : "0px",
            "--dock-right": panels.right ? "250px" : "0px",
            "--dock-bottom": panels.bottom ? "220px" : "0px",
          } as CSSProperties
        }
      >
      <div className="workspace">
        {/*
          A closed panel is gone, not a strip: its toggle is a tab on the
          stage's edge (`DockToggle`), where it stays whether the panel is
          open or closed (M27). Greyed out and dead to the pointer while a
          capture records or previews (spec.md 8.7, M26): every write is
          refused then, and a panel that looks live but refuses is worse than
          one that says it is off.
        */}
        {panels.left ? (
          <aside className={recording !== null ? "sidebar left dimmed" : "sidebar left"}>
            <LayerPanel
              project={project}
              step={step}
              selection={selection}
              activeLayer={activeLayer}
              onSelect={selectObjects}
              onActivateLayer={setActiveLayer}
              onActiveKind={setActiveKind}
              onChanged={setProject}
              viewBounds={viewBounds}
            />
          </aside>
        ) : (
          <span className="dock-space" />
        )}

        <MapView
          ref={mapRef}
          project={project}
          step={step}
          selection={selection}
          activeLayer={activeLayer}
          picking={picking}
          onPicked={() => setPicking(null)}
          onProjectChanged={setProject}
          settings={settings}
          onSettings={setSettings}
          onRecording={setRecording}
          onStepChange={setStep}
          onSelect={selectObjects}
          onViewport={setViewport}
          autoKey={autoKey}
          viewSlot={viewSlot}
          activeKind={activeKind}
          libraryRevision={libraryRevision}
          shapeEditing={shapeEditing}
          onExitShapeEditing={exitShapeEditing}
        />

        {panels.right ? (
          <aside className={recording !== null ? "sidebar right dimmed" : "sidebar right"}>
              <section className="panel-section">
                <header onClick={() => toggle("properties")}>
                  <span className="disclose">{panels.properties ? "▾" : "▸"}</span>
                  <h2>Properties</h2>
                </header>
                {panels.properties && (
                  <Inspector
                    project={project}
                    selection={selection}
                    step={step}
                    activeLayer={activeLayer}
                    autoKey={autoKey}
                    picking={picking}
                    onPick={setPicking}
                    onChanged={setProject}
                  />
                )}
              </section>
              <section className="panel-section">
                <header onClick={() => toggle("history")}>
                  <span className="disclose">{panels.history ? "▾" : "▸"}</span>
                  <h2>History</h2>
                </header>
                {panels.history && <HistoryPanel project={project} onChanged={setProject} />}
              </section>
          </aside>
        ) : (
          <span className="dock-space" />
        )}
      </div>

      {/*
        Mounted whether or not it is shown: its playback loop is what plays a
        macro preview (spec.md 8.7), and it has to keep playing with the
        timeline put away (M27).
      */}
      <Timeline
          hidden={!panels.bottom}
          project={project}
        step={step}
        onStepChange={setStep}
        selection={selection}
        onSelect={selectObjects}
        viewport={viewport}
        playback={playback}
        autoKey={autoKey}
        onAutoKey={setAutoKey}
        onChanged={setProject}
        onFramesSelected={onFramesSelected}
        onKeysSelected={onKeysSelected}
        settings={settings}
        capture={recording}
        onCapture={(mode) => mapRef.current?.setCapture(mode)}
        shapeEditing={shapeEditing}
        onShapeEditing={toggleShapeEditing}
      />

      {/*
        The three tabs sit on the stage's edges, on the border between a
        panel and the map, and do not move when the panel opens or closes:
        the side ones are centred top to bottom, the timeline's left to
        right (M27). Before this each toggle was a strip inside its panel
        that jumped from the panel's top to its middle as it folded.
      */}
      <DockToggle
        side="left"
        open={panels.left}
        label="the layer panel"
        onToggle={() => toggle("left")}
      />
      <DockToggle
        side="right"
        open={panels.right}
        label="the properties and history"
        onToggle={() => toggle("right")}
      />
      <DockToggle
        side="bottom"
        open={panels.bottom}
        label="the timeline"
        onToggle={() => toggle("bottom")}
      />
      </div>

      {showSettings && settings !== null && (
        <SettingsDialog
          settings={settings}
          project={project}
          onSettings={setSettings}
          onProject={setProject}
          onLibrary={() => setLibraryRevision((at) => at + 1)}
          onClose={() => setShowSettings(false)}
        />
      )}

      {exporting && (
        <ExportDialog project={project} onClose={() => setExporting(false)} />
      )}

      {creating !== null && (
        <NewProjectDialog
          discardUnsaved={creating.discardUnsaved}
          onCreated={(created) => {
            setCreating(null);
            setSelection([]);
            setShapeEditing(null);
            setActiveLayer(null);
            setStep(0);
            reportError(null);
            setProject(created);
          }}
          onCancel={() => setCreating(null)}
        />
      )}

      {askUnsaved !== null && (
        <UnsavedChangesDialog name={askUnsaved.name} onChoose={answerUnsaved} />
      )}

      <div className="statusbar">
        <BusySpinner />
        <HistoryProgressBar />
        {info && (
          <>
            <span className="muted">v{info.version}</span>
            <span className="muted">
              evaluator: <span className="accent">{info.evaluator.backend}</span>
              {info.evaluator.fallback_reason !== null &&
                ` — ${info.evaluator.fallback_reason}`}
            </span>
          </>
        )}
        <span className="muted">
          {project.grid_ni} × {project.grid_nj} grid
        </span>
        <span className="spacer" />
        <StatusHint status={status} />
        <span className="spacer" />
        {project.path !== null && <span className="muted path">{project.path}</span>}
        {/*
          The product of the whole app, so it sits apart from the file
          operations in the title bar rather than among them.
        */}
        <button className="export" onClick={() => setExporting(true)}>
          Export GRIB…
        </button>
      </div>
    </div>
    </UnitsProvider>
  );
}

/** The glyph a dock tab shows: pointing into the panel to open it, out to close it. */
const DOCK_GLYPH = {
  left: { open: "◀", closed: "▶" },
  right: { open: "▶", closed: "◀" },
  bottom: { open: "▼", closed: "▲" },
} as const;

/**
 * A panel's toggle: a tab on the stage's edge, on the border between the
 * panel and the map (M27). Positioned by the stage's dock variables, so it
 * sits on the panel's inner edge while the panel is open and on the window's
 * edge while it is closed, centred along that edge either way.
 */
function DockToggle({
  side,
  open,
  label,
  onToggle,
}: {
  side: "left" | "right" | "bottom";
  open: boolean;
  label: string;
  onToggle: () => void;
}) {
  const verb = open ? "Hide" : "Show";
  return (
    <button
      className={`dock-toggle ${side}`}
      onClick={onToggle}
      title={`${verb} ${label}`}
      aria-label={`${verb} ${label}`}
      aria-expanded={open}
    >
      {DOCK_GLYPH[side][open ? "open" : "closed"]}
    </button>
  );
}

/**
 * The status bar's spinner, left of the version: turning while a long task
 * runs — a GRIB or image import, an open, a save, an export, a capture's
 * bake — and invisible otherwise. The ipc layer decides which commands count,
 * so a new long command needs a label there and nothing here. Its own
 * component, subscribed to the busy store, so the shell does not re-render
 * when work starts and ends.
 */
function BusySpinner() {
  const busy = useBusy();
  const on = isBusy(busy);
  return (
    <span
      className={on ? "busy-spinner on" : "busy-spinner"}
      role="status"
      aria-live="polite"
      aria-label={on ? busy.labels.join(", ") : "Idle"}
      title={on ? busy.labels.join(" · ") : undefined}
    />
  );
}

/**
 * How far a history fetch has got (spec.md 4.10, M38).
 *
 * A fetch is minutes of network with nothing else to look at, and a spinner
 * that only turns cannot tell a slow archive from a stalled one. The backend
 * knows how many steps it will read before it reads the first, so this is a
 * real fraction and not an animation.
 *
 * It is bounded by the busy store rather than by the last event: an import
 * that fails leaves its final progress behind, and the bar has to go when the
 * command does, whichever way it ended.
 *
 * Its own component, and its own subscription, so a progress event re-renders
 * this bar and not the shell.
 */
function HistoryProgressBar() {
  const busy = useBusy();
  const [progress, setProgress] = useState<HistoryProgress | null>(null);
  const running = busy.labels.includes(HISTORY_LABEL);

  useEffect(() => {
    const pending = listen<HistoryProgress>("history://progress", (event) => {
      setProgress(event.payload);
    });
    return () => {
      void pending.then((unlisten) => unlisten());
    };
  }, []);

  // Cleared on the way out, so the next fetch does not open on the last
  // one's bar before its first event lands.
  useEffect(() => {
    if (!running) setProgress(null);
  }, [running]);

  if (!running) return null;
  const total = progress?.total ?? 0;
  const percent = total > 0 ? Math.round(((progress?.done ?? 0) / total) * 100) : 0;
  const label =
    progress === null
      ? "Opening the archives"
      : `${progress.archive} · ${progress.done}/${progress.total}`;
  return (
    <span className="history-progress" title={label} aria-label={label}>
      <span className="progress-bar">
        <span className="progress-fill" style={{ width: `${percent}%` }} />
      </span>
      <span className="muted">{label}</span>
    </span>
  );
}

/**
 * The status bar's middle: the tool's hint, an error, or a flash (M25).
 *
 * Its own component so the hint store's updates — which arrive per pointer
 * report from the map — re-render this span and nothing else.
 */
function StatusHint({ status }: { status: string | null }) {
  const state = useHint();
  const line = shown(state);
  // The map's activity — tiles still rendering — leads the line (M27).
  const activity =
    state.activity !== null ? <span className="activity">{state.activity}</span> : null;
  if (status !== null) {
    return (
      <span className="hint accent">
        {activity}
        {activity && " · "}
        {status}
      </span>
    );
  }
  if (line === null) return <span className="hint">{activity}</span>;
  return (
    <span
      className={`${line.kind === "error" ? "hint error" : "hint muted"}${line.kind === "error" && state.retry ? " has-retry" : ""}`}
      title={line.detail ?? undefined}
    >
      <span className="hint-message">{activity}{activity && " · "}{line.text}</span>
      {line.kind === "error" && state.retry && <button className="status-retry" onClick={retryError}>Retry download</button>}
    </span>
  );
}

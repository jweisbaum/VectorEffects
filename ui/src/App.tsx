import { useCallback, useEffect, useRef, useState } from "react";

import ExportDialog from "./project/ExportDialog";
import HistoryPanel from "./panels/HistoryPanel";
import Inspector from "./panels/Inspector";
import LayerPanel from "./panels/LayerPanel";
import MapView, { type MapHandle } from "./map/MapView";
import Timeline from "./timeline/Timeline";
import NewProjectDialog from "./project/NewProjectDialog";
import StartScreen from "./project/StartScreen";
import UnsavedChangesDialog from "./project/UnsavedChangesDialog";
import { mayReplaceProject, type UnsavedChoice } from "./project/saveGuard";
import { pickProjectToOpen, pickProjectToSave } from "./project/dialogs";
import { api, IpcError } from "./ipc";
import type { AppInfo } from "./generated/AppInfo";
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
  const [info, setInfo] = useState<AppInfo | null>(null);
  const [project, setProject] = useState<ProjectSummary | null>(null);
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
  /**
   * Whether the timeline has a GRIB frame selected, in which case copy and
   * paste belong to it and not to the object clipboard (spec.md 4.8, M20).
   * A ref rather than state: the key handler reads it and nothing draws it.
   */
  const framesSelected = useRef(false);
  const onFramesSelected = useCallback((active: boolean) => {
    framesSelected.current = active;
  }, []);
  // Which layer receives new objects, and what a plain marquee is scoped to
  // (spec.md 6.1, 8.2). Null means the top of the stack.
  const [activeLayer, setActiveLayer] = useState<number | null>(null);
  // Armed by the inspector, answered by the map: the next map click places this
  // property of this object.
  const [picking, setPicking] = useState<PositionPick | null>(null);
  const [error, setError] = useState<string | null>(null);
  // Auto-key (spec.md 9.3): while on, an edit in the inspector keys the
  // current step instead of changing the property's base.
  const [autoKey, setAutoKey] = useState(false);
  // The map's visible tiles, which the timeline renders ahead for and asks
  // readiness about (spec.md 9.5). The map reports them; nothing else knows.
  const [viewport, setViewport] = useState<TileAddress[]>([]);
  const mapRef = useRef<MapHandle | null>(null);
  // The map holds the tiles; playback asks it, not the backend, whether a step
  // can be shown (spec.md 9.4). Before the map exists there is nothing to wait for.
  const warm = useCallback((target: number) => mapRef.current?.warm(target) ?? true, []);

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

  const report = (err: unknown) =>
    setError(err instanceof IpcError ? `[${err.kind}] ${err.message}` : String(err));

  const flash = (message: string) => {
    setStatus(message);
    setError(null);
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
      const opened = await api.openProject(path, decision.discardUnsaved);
      setSelection([]);
      setActiveLayer(null);
      setStep(0);
      setError(null);
      setProject(opened);
    } catch (err) {
      report(err);
    }
  }, [mayReplace]);

  // Standard shortcuts, so saving does not require reaching for the toolbar.
  const modal = exporting || creating !== null || askUnsaved !== null;
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      // A dialog is a decision in progress; undoing or saving behind it would
      // change the very thing being decided about.
      if (modal) return;
      const accel = event.metaKey || event.ctrlKey;
      if (!accel) return;
      const key = event.key.toLowerCase();

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
      } else if (key === "c" && selection.length > 0) {
        event.preventDefault();
        void api.copyObjects(selection, step).catch(report);
      } else if (key === "x" && selection.length > 0) {
        event.preventDefault();
        void api
          .cutObjects(selection, step)
          .then((next) => {
            setSelection([]);
            setProject(next);
          })
          .catch(report);
      } else if (key === "v") {
        event.preventDefault();
        // Shift pastes at the original step numbers instead of moving the
        // animation to the current one (spec.md 8.5).
        void api.pasteObjects(null, step, event.shiftKey).then(setProject).catch(report);
      } else if (key === "z") {
        event.preventDefault();
        void (event.shiftKey ? api.redo() : api.undo())
          .then(setProject)
          .catch(report);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [modal, openProject, save, saveAs, selection, startNewProject, step]);

  if (!project) {
    return <StartScreen onOpened={setProject} />;
  }

  return (
    <div className="app">
      <div className="titlebar">
        <span className="brand">VectorEffects</span>
        <span className="project-name">
          {project.name}
          {project.dirty && <span className="dirty" title="Unsaved changes"> •</span>}
        </span>
        <span className="muted project-meta">
          {project.field_kind} · {project.resolution_label} · {project.step_count} ×{" "}
          {project.step_hours} h
        </span>
        <span className="spacer" />
        <button onClick={() => void startNewProject()}>New…</button>
        <button onClick={() => void openProject()}>Open…</button>
        <button onClick={() => void save()}>Save</button>
        <button onClick={() => void saveAs()}>Save As…</button>
      </div>

      <div className="workspace">
        <aside className="sidebar left">
          <LayerPanel
            project={project}
            step={step}
            selection={selection}
            activeLayer={activeLayer}
            onSelect={setSelection}
            onActivateLayer={setActiveLayer}
            onChanged={setProject}
          />
        </aside>

        <MapView
          ref={mapRef}
          project={project}
          step={step}
          selection={selection}
          activeLayer={activeLayer}
          picking={picking}
          onPicked={() => setPicking(null)}
          onProjectChanged={setProject}
          onStepChange={setStep}
          onSelect={setSelection}
          onViewport={setViewport}
          autoKey={autoKey}
        />

        <aside className="sidebar right">
          <Inspector
            project={project}
            selection={selection}
            step={step}
            autoKey={autoKey}
            picking={picking}
            onPick={setPicking}
            onChanged={setProject}
          />
          <HistoryPanel project={project} onChanged={setProject} />
        </aside>
      </div>

      <Timeline
        project={project}
        step={step}
        onStepChange={setStep}
        selection={selection}
        onSelect={setSelection}
        viewport={viewport}
        warm={warm}
        autoKey={autoKey}
        onAutoKey={setAutoKey}
        onChanged={setProject}
        onFramesSelected={onFramesSelected}
      />

      {exporting && (
        <ExportDialog project={project} onClose={() => setExporting(false)} />
      )}

      {creating !== null && (
        <NewProjectDialog
          discardUnsaved={creating.discardUnsaved}
          onCreated={(created) => {
            setCreating(null);
            setSelection([]);
            setActiveLayer(null);
            setStep(0);
            setError(null);
            setProject(created);
          }}
          onCancel={() => setCreating(null)}
        />
      )}

      {askUnsaved !== null && (
        <UnsavedChangesDialog name={askUnsaved.name} onChoose={answerUnsaved} />
      )}

      <div className="statusbar">
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
        {status !== null && <span className="accent">{status}</span>}
        {error !== null && <span className="error">{error}</span>}
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
  );
}

import { useEffect, useState } from "react";

import { api, IpcError } from "../ipc";
import type { NewProjectRequest } from "../generated/NewProjectRequest";
import type { ProjectSummary } from "../generated/ProjectSummary";
import type { Autosave } from "../generated/Autosave";
import type { RecentProject } from "../generated/RecentProject";
import NewProjectForm from "./NewProjectForm";
import { pickGribToImport, pickProjectToOpen } from "./dialogs";

/**
 * Shown when no project is open.
 *
 * The settings form is shared with the New Project dialog (`NewProjectForm`),
 * which explains why the immutable-settings warning lives there rather than
 * here.
 */
export default function StartScreen({
  onOpened,
}: {
  onOpened: (project: ProjectSummary) => void;
}) {
  const [recent, setRecent] = useState<RecentProject[]>([]);
  /** Unsaved work an unclean shutdown left behind (spec.md 4.2, M10). */
  const [autosaves, setAutosaves] = useState<Autosave[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    api.recentProjects().then(setRecent).catch(() => setRecent([]));
    api.autosaves().then(setAutosaves).catch(() => setAutosaves([]));
  }, []);

  const run = async (action: () => Promise<ProjectSummary>) => {
    setBusy(true);
    setError(null);
    try {
      onOpened(await action());
    } catch (err) {
      setError(err instanceof IpcError ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  };

  const create = (request: NewProjectRequest) => void run(() => api.newProject(request));

  const openFrom = (path: string) => run(() => api.openProject(path));

  const browse = async () => {
    const path = await pickProjectToOpen();
    if (path !== null) await openFrom(path);
  };

  /** A project shaped by a GRIB2 file: its kind, grid, step and span (spec 4.8). */
  const openFromGrib = async () => {
    const path = await pickGribToImport();
    if (path !== null) await run(() => api.newProjectFromGrib(path));
  };

  return (
    <div className="start">
      <div className="start-panel">
        <header>
          <h1>VectorEffects</h1>
          <p className="muted">
            Paint global wind and current fields. Export GRIB2.
          </p>
        </header>

        <section className="start-new">
          <h2>New project</h2>
          <NewProjectForm disabled={busy} submitLabel="Create project" onSubmit={create} />
        </section>

        {autosaves.length > 0 && (
          <section className="start-recover">
            <h2>Recovered work</h2>
            <p className="muted">
              The application did not close cleanly. These are snapshots of unsaved work,
              taken every minute or fifty edits; recovering one opens it as the project it
              came from, unsaved.
            </p>
            <ul className="recent">
              {autosaves.map((entry) => (
                <li key={entry.id}>
                  <button
                    className="recent-item"
                    disabled={busy}
                    onClick={() => void run(() => api.recoverAutosave(entry.id, false))}
                    title={entry.original_path ?? "never saved"}
                  >
                    <span className="recent-name">{entry.name}</span>
                    <span className="recent-path muted">
                      {entry.original_path ?? "never saved"} ·{" "}
                      {new Date(entry.saved_unix_s * 1000).toLocaleString()}
                    </span>
                  </button>
                  <button
                    disabled={busy}
                    onClick={() =>
                      void api.discardAutosave(entry.id).then(setAutosaves).catch(() => undefined)
                    }
                    title="Drop this snapshot"
                  >
                    Discard
                  </button>
                </li>
              ))}
            </ul>
          </section>
        )}

        <section className="start-open">
          <h2>Open</h2>
          <button onClick={browse} disabled={busy}>
            Browse…
          </button>
          <button onClick={() => void openFromGrib()} disabled={busy} title="Create a project from a GRIB2 file: the field kind, grid, time step and span come from the file">
            Open from GRIB…
          </button>

          {recent.length > 0 && (
            <>
              <h3 className="muted">Recent</h3>
              <ul className="recent">
                {recent.map((entry) => (
                  <li key={entry.path}>
                    <button
                      className="recent-item"
                      disabled={busy || !entry.exists}
                      onClick={() => void openFrom(entry.path)}
                      title={entry.path}
                    >
                      <span className="recent-name">{entry.name}</span>
                      <span className="recent-path muted">{entry.path}</span>
                      {!entry.exists && <span className="error"> missing</span>}
                    </button>
                  </li>
                ))}
              </ul>
            </>
          )}
        </section>

        {error !== null && <p className="error">{error}</p>}
      </div>
    </div>
  );
}

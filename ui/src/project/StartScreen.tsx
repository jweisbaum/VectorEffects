import { useEffect, useState } from "react";

import { api, IpcError } from "../ipc";
import type { NewProjectRequest } from "../generated/NewProjectRequest";
import type { ProjectSummary } from "../generated/ProjectSummary";
import type { RecentProject } from "../generated/RecentProject";
import NewProjectForm from "./NewProjectForm";
import { pickProjectToOpen } from "./dialogs";

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
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    api.recentProjects().then(setRecent).catch(() => setRecent([]));
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

        <section className="start-open">
          <h2>Open</h2>
          <button onClick={browse} disabled={busy}>
            Browse…
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

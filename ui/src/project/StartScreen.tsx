import { useEffect, useState } from "react";

import { api, IpcError } from "../ipc";
import type { NewProjectRequest } from "../generated/NewProjectRequest";
import type { ProjectSummary } from "../generated/ProjectSummary";
import type { Autosave } from "../generated/Autosave";
import type { RecentProject } from "../generated/RecentProject";
import NewProjectForm from "./NewProjectForm";
import ConfirmDialog from "./ConfirmDialog";
import type { AppSettings } from "../generated/AppSettings";
import HelpMenu from "../help/HelpMenu";
import LanguagePicker from "../i18n/LanguagePicker";
import { useT } from "../i18n";
import { pickGribToImport, pickProjectToOpen, pickZarrToImport } from "./dialogs";

/**
 * Shown when no project is open.
 *
 * The settings form is shared with the New Project dialog (`NewProjectForm`),
 * which explains why the immutable-settings warning lives there rather than
 * here.
 */
export default function StartScreen({
  onOpened,
  onSettings,
  onPreferences,
}: {
  onOpened: (project: ProjectSummary) => void;
  onSettings?: () => void;
  /** The settings after the language picker saved them. */
  onPreferences?: (settings: AppSettings) => void;
}) {
  const t = useT();
  const [recent, setRecent] = useState<RecentProject[]>([]);
  /** Unsaved work an unclean shutdown left behind (spec.md 4.2, M10). */
  const [autosaves, setAutosaves] = useState<Autosave[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  /** Whether the confirmation for clearing the recent list is up. */
  const [askClearRecent, setAskClearRecent] = useState(false);

  useEffect(() => {
    api.recentProjects().then((entries) => setRecent(entries.filter((entry) => entry.exists))).catch(() => setRecent([]));
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

  const openFromZarr = async () => {
    const path = await pickZarrToImport();
    if (path !== null) await run(() => api.newProjectFromZarr(path));
  };

  /**
   * Drops every recent entry. The list is replaced with what the backend
   * reports rather than an assumed empty one, so a refused write shows.
   */
  const clearRecent = async () => {
    setAskClearRecent(false);
    setError(null);
    try {
      setRecent(await api.clearRecentProjects());
    } catch (err) {
      setError(err instanceof IpcError ? err.message : String(err));
    }
  };

  return (
    <div className="start">
      <div className="start-panel">
        <header>
          <div className="start-header-controls">
            <LanguagePicker onSettings={onPreferences} />
            <HelpMenu />
            {onSettings && <button className="start-settings" data-feature="start:settings" onClick={onSettings}>{t("Settings")}</button>}
          </div>
          <h1>VectorEffects</h1>
          <p className="muted">
            {t("Paint global wind and current fields. Export GRIB2.")}
          </p>
        </header>

        <section className="start-new" data-feature="start:new">
          <h2>{t("New project")}</h2>
          <NewProjectForm disabled={busy} submitLabel={t("Create project")} onSubmit={create} />
        </section>

        {autosaves.length > 0 && (
          <section className="start-recover" data-feature="start:recover">
            <h2>{t("Recovered work")}</h2>
            <p className="muted">
              {t("The application did not close cleanly. These are snapshots of unsaved work, taken every minute or fifty edits; recovering one opens it as the project it came from, unsaved.")}
            </p>
            <ul className="recent">
              {autosaves.map((entry) => (
                <li key={entry.id}>
                  <button
                    className="recent-item"
                    disabled={busy}
                    onClick={() => void run(() => api.recoverAutosave(entry.id, false))}
                    title={entry.original_path ?? t("never saved")}
                  >
                    <span className="recent-name">{entry.name}</span>
                    <span className="recent-path muted">
                      {entry.original_path ?? t("never saved")} ·{" "}
                      {new Date(entry.saved_unix_s * 1000).toLocaleString()}
                    </span>
                  </button>
                  <button
                    disabled={busy}
                    onClick={() =>
                      void api.discardAutosave(entry.id).then(setAutosaves).catch(() => undefined)
                    }
                    title={t("Drop this snapshot")}
                  >
                    {t("Discard")}
                  </button>
                </li>
              ))}
            </ul>
          </section>
        )}

        <section className="start-open">
          <h2>{t("Open")}</h2>
          <button onClick={browse} disabled={busy} data-feature="start:browse">
            {t("Browse…")}
          </button>
          <button onClick={() => void openFromGrib()} disabled={busy} data-feature="start:open-grib"
            title={t("Create a project from a GRIB2 file: the field kind, grid, time step and span come from the file")}>
            {t("Open from GRIB…")}
          </button>
          <button onClick={() => void openFromZarr()} disabled={busy} data-feature="start:open-zarr"
            title={t("Create a project from a routing Zarr directory: wind, currents, grid and times come from the store")}>
            {t("Open from Zarr…")}
          </button>

          {recent.length > 0 && (
            <>
              <div className="recent-header">
                <h3 className="muted">{t("Recent")}</h3>
                <button
                  disabled={busy}
                  data-feature="start:clear-recent"
                  onClick={() => setAskClearRecent(true)}
                  title={t("Forget every project in this list. The projects themselves are not deleted.")}
                >
                  {t("Clear")}
                </button>
              </div>
              <ul className="recent" data-feature="start:recent">
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
                      {!entry.exists && <span className="error"> {t("missing")}</span>}
                    </button>
                  </li>
                ))}
              </ul>
            </>
          )}
        </section>

        {error !== null && <p className="error">{error}</p>}
      </div>

      {askClearRecent && (
        <ConfirmDialog
          title={t("Clear recent projects")}
          body={t("This forgets the list of recently opened projects. The projects themselves are not deleted.")}
          confirmLabel={t("Clear")}
          onConfirm={() => void clearRecent()}
          onCancel={() => setAskClearRecent(false)}
        />
      )}
    </div>
  );
}

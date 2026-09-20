//! Crash recovery: a snapshot of the open project, taken while it is dirty
//! (spec.md 4.2, M10).
//!
//! The application has always had an `autosave` directory and never written
//! to it. This is the writer. A thread looks at the open project every
//! [`TICK`]; if it is dirty, has changed since the last snapshot, and either
//! [`INTERVAL`] has passed or [`ENTRIES`] history entries have been made since
//! — whichever comes first, as spec.md 4.2 says — the document is cloned out
//! from under the session lock and saved to `autosave/<project id>.veproj`
//! beside a small manifest naming where the project came from. A clean save
//! or a deliberate close removes the snapshot; a crash leaves it, and the
//! start screen offers it back.
//!
//! **Cloned, then saved, never saved under the lock.** `io::save` writes a
//! whole archive — captures included — and a large project takes long enough
//! that holding the session for it would stall every edit at the moment the
//! user is editing. The export makes the same choice for the same reason.
//!
//! **A snapshot is never a source.** Recovering one opens it *as the original
//! project*, dirty, with the original path if there was one: the next Save
//! writes where the user meant to save, and the snapshot goes when it does.
//! Nothing reads a snapshot for any other purpose, and nothing but a crash
//! keeps one.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use ve_core::io;

use crate::commands::AppState;
use crate::error::{AppError, Context, Result};
use crate::projects::{ProjectSummary, refuse_to_discard, with_session};
use crate::session::OpenProject;
use crate::settings::AutosaveMode;

/// How often the thread looks.
///
/// Often enough that the entry-count trigger fires within a few seconds of
/// the fiftieth edit; the look itself is a lock and two comparisons.
pub const TICK: Duration = Duration::from_secs(5);

/// The longest a dirty project goes between snapshots (spec.md 4.2).
///
/// A minute bounds what a crash can cost. Not shorter, because a snapshot is a
/// full archive write and a 5,000-object project with captures is tens of
/// megabytes; not on every edit, because a brush stroke is an edit and the
/// disk would never be idle.
pub const INTERVAL: Duration = Duration::from_secs(60);

/// The most history entries that go by between snapshots (spec.md 4.2).
///
/// A minute of fast painting is far more than fifty entries; this is the
/// trigger for the burst, the interval for the pause.
pub const ENTRIES: usize = 50;

/// What the manifest beside a snapshot records.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Manifest {
    /// The project's display name.
    name: String,
    /// Where the project lived, if it had been saved.
    original_path: Option<PathBuf>,
    /// When the snapshot was taken, seconds since the epoch.
    saved_unix_s: u64,
    /// The document revision the snapshot was taken at, so an unchanged
    /// project is not written again.
    revision: u64,
    /// How many history entries the project had, for the fifty-entry trigger.
    entries: usize,
}

/// A snapshot the start screen can offer back.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "Autosave.ts")]
pub struct Autosave {
    /// The project id, which is also the snapshot's file stem.
    pub id: u64,
    /// The project's display name.
    pub name: String,
    /// Where it lived, if it had been saved; null for a project never saved.
    pub original_path: Option<String>,
    /// When the snapshot was taken, seconds since the epoch.
    pub saved_unix_s: u64,
}

fn snapshot_path(dir: &Path, id: u64) -> PathBuf {
    dir.join(format!("{id}.veproj"))
}

fn manifest_path(dir: &Path, id: u64) -> PathBuf {
    dir.join(format!("{id}.json"))
}

fn now_unix_s() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Takes a snapshot of the open project if one is due.
///
/// Due means: dirty, changed since the last snapshot, and either [`INTERVAL`]
/// has passed since it or [`ENTRIES`] history entries have been made since —
/// or there is no snapshot yet at all. `force` skips the two timers, for the
/// thread's first look at a project and for tests; the dirty-and-changed
/// checks are never skipped, because a snapshot of nothing new is a write for
/// nothing.
///
/// The thread's body, callable without a Tauri handle. Returns whether a
/// snapshot was written.
pub fn snapshot(state: &AppState, force: bool) -> Result<bool> {
    let dir = &state.paths.autosave_dir;
    // Clone the document out from under the lock (see the module doc).
    let Some((project, path, revision, entries, mode)) = with_session(state, |session| {
        let mode = session.settings.autosave;
        Ok(session.open.as_ref().and_then(|open| {
            open.dirty.then(|| {
                (
                    open.project.clone(),
                    open.path.clone(),
                    open.revision,
                    open.history.entries().len(),
                    mode,
                )
            })
        }))
    })?
    else {
        return Ok(false);
    };
    // Off means nothing is written until the user saves (D70).
    if mode == AutosaveMode::Off {
        return Ok(false);
    }

    let id = project.id.raw();
    let previous = std::fs::read_to_string(manifest_path(dir, id))
        .ok()
        .and_then(|text| serde_json::from_str::<Manifest>(&text).ok());
    if let Some(previous) = &previous {
        if previous.revision == revision {
            return Ok(false);
        }
        let elapsed = now_unix_s().saturating_sub(previous.saved_unix_s);
        let burst = entries.saturating_sub(previous.entries) >= ENTRIES;
        if !force && elapsed < INTERVAL.as_secs() && !burst {
            return Ok(false);
        }
    }

    std::fs::create_dir_all(dir).doing("make the autosave folder at", dir.display())?;
    // In `Save` mode a project with a path is written in place instead — the
    // same cadence, the file the user chose. The save clears the snapshot,
    // so the manifest is written afterwards to carry the cadence: a manifest
    // without a snapshot file lists nothing on the start screen and only
    // says when the last write was. A project with no path yet has nowhere
    // to be written and takes a snapshot like everyone else.
    if mode == AutosaveMode::Save && path.is_some() {
        crate::projects::save(state)?;
        tracing::info!(id, "autosaved the project in place");
    } else {
        io::save(&project, &snapshot_path(dir, id)).doing(
            "write a recovery snapshot to",
            snapshot_path(dir, id).display(),
        )?;
        tracing::info!(id, "wrote a crash-recovery snapshot");
    }
    let manifest = Manifest {
        name: project.name.clone(),
        original_path: path,
        saved_unix_s: now_unix_s(),
        revision,
        entries,
    };
    std::fs::write(
        manifest_path(dir, id),
        serde_json::to_string_pretty(&manifest).map_err(ve_core::CoreError::Json)?,
    )
    .doing(
        "write the autosave record to",
        manifest_path(dir, id).display(),
    )?;
    Ok(true)
}

/// Removes the snapshot of a project, if there is one.
///
/// Called on a clean save and on a deliberate close: what the user saved or
/// chose to drop is not something to offer back.
pub fn forget(state: &AppState, id: u64) {
    let dir = &state.paths.autosave_dir;
    let _ = std::fs::remove_file(snapshot_path(dir, id));
    let _ = std::fs::remove_file(manifest_path(dir, id));
}

/// Every snapshot on disk, newest first.
pub fn list(state: &AppState) -> Vec<Autosave> {
    let dir = &state.paths.autosave_dir;
    let mut out: Vec<Autosave> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            if path.extension()? != "json" {
                return None;
            }
            let id: u64 = path.file_stem()?.to_str()?.parse().ok()?;
            if !snapshot_path(dir, id).exists() {
                return None;
            }
            let manifest: Manifest =
                serde_json::from_str(&std::fs::read_to_string(&path).ok()?).ok()?;
            Some(Autosave {
                id,
                name: manifest.name,
                original_path: manifest
                    .original_path
                    .map(|p| p.to_string_lossy().into_owned()),
                saved_unix_s: manifest.saved_unix_s,
            })
        })
        .collect();
    out.sort_by_key(|entry| std::cmp::Reverse(entry.saved_unix_s));
    out
}

/// Opens a snapshot as the project it was taken from.
///
/// Dirty, with the original path if there was one, so the next Save writes
/// where the user meant to save — and the snapshot goes when it does. A
/// project that had never been saved comes back with no path, and Save asks.
pub fn recover(state: &AppState, id: u64, discard_unsaved: bool) -> Result<ProjectSummary> {
    let dir = &state.paths.autosave_dir;
    let manifest: Manifest = std::fs::read_to_string(manifest_path(dir, id))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .ok_or_else(|| AppError::BadOption {
            field: "autosave",
            value: "no such snapshot".to_owned(),
        })?;
    let mut opening = state.opening.begin();
    opening.document("the recovery snapshot");
    let mut project = io::load(&snapshot_path(dir, id)).doing(
        "reopen the recovery snapshot at",
        snapshot_path(dir, id).display(),
    )?;
    crate::import::attach_rasters(&mut project, &mut opening);
    opening.finished();

    with_session(state, |session| {
        refuse_to_discard(session, discard_unsaved)?;
        let mut open = match manifest.original_path.clone() {
            Some(path) => OpenProject::loaded(project, path),
            None => OpenProject::created(project),
        };
        open.dirty = true;
        session.open = Some(open);
        tracing::info!(id, "recovered a project from its snapshot");
        Ok(ProjectSummary::of(session.require_open()?))
    })
}

/// Starts the snapshot thread for the life of the process.
pub fn start(app: tauri::AppHandle) {
    std::thread::Builder::new()
        .name("autosave".to_owned())
        .spawn(move || {
            use tauri::Manager;
            loop {
                std::thread::sleep(TICK);
                let state = app.state::<AppState>();
                if let Err(err) = snapshot(&state, false) {
                    tracing::warn!(%err, "crash-recovery snapshot failed");
                }
            }
        })
        .map(drop)
        .unwrap_or_else(|err| tracing::error!(%err, "could not start the autosave thread"));
}

/// The snapshots on disk, for the start screen.
#[tauri::command]
pub fn autosaves(state: tauri::State<'_, AppState>) -> Result<Vec<Autosave>> {
    Ok(list(&state))
}

/// Opens a snapshot as the project it was taken from.
#[tauri::command(async)]
pub fn recover_autosave(
    state: tauri::State<'_, AppState>,
    id: u64,
    discard_unsaved: bool,
) -> Result<ProjectSummary> {
    recover(&state, id, discard_unsaved)
}

/// Drops a snapshot the user does not want back.
#[tauri::command]
pub fn discard_autosave(state: tauri::State<'_, AppState>, id: u64) -> Result<Vec<Autosave>> {
    forget(&state, id);
    Ok(list(&state))
}

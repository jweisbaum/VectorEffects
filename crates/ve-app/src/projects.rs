//! Project lifecycle commands: new, open, save, close.
//!
//! The frontend never sees `ve-core` types directly. Everything crossing IPC is
//! a flat summary with enums as strings, which keeps `ts-rs` out of the core
//! crate and keeps the wire format readable.

use std::path::PathBuf;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use ve_core::io;
use ve_core::project::{FieldKind, Project, ProjectSettings, Resolution, StepHours};
use ve_core::vector::DirectionConvention;

use crate::commands::AppState;
use crate::error::{AppError, Context, Result};
use crate::session::{OpenProject, Session};

/// What the frontend needs to know about the open project.
#[derive(Debug, Clone, Serialize, JsonSchema, TS)]
#[ts(export, export_to = "ProjectSummary.ts")]
pub struct ProjectSummary {
    /// Display name.
    pub name: String,
    /// Where it lives on disk; absent until first saved.
    pub path: Option<String>,
    /// Whether there are changes not yet written to disk.
    pub dirty: bool,
    /// `"wind"` or `"current"`.
    pub field_kind: String,
    /// Grid spacing in degrees.
    pub resolution_deg: f64,
    /// Human-readable grid spacing, e.g. `"0.25°"`.
    pub resolution_label: String,
    /// Grid columns.
    pub grid_ni: u32,
    /// Grid rows.
    pub grid_nj: u32,
    /// Hours between time steps.
    pub step_hours: u32,
    /// Number of time steps.
    pub step_count: u32,
    /// When step 0 is, as seconds since the Unix epoch, if set (spec.md 9.1).
    pub start_unix_s: Option<i64>,
    /// `"from"` or `"toward"`: how directions are shown (spec.md 3.3).
    pub direction_convention: String,
    /// Knots at the top of the speed colour ramp when the map shows wind
    /// (spec.md 5.3, M15), and when it shows current (M29).
    ///
    /// The project's, so two people opening one file see the same map. Tiles
    /// carry speed and not colour, so changing either costs no render.
    pub wind_scale_knots: f64,
    pub current_scale_knots: f64,
    /// The gradient the wind layers are painted with (spec.md 5.3, M42), by
    /// identifier: `colour_gradients` says what the identifiers mean.
    pub wind_gradient: String,
    /// And the current layers, which start on a different one.
    pub current_gradient: String,
    /// The kinds of field the visible layers hold — `"wind"`, `"current"` —
    /// wind first: what an export writes, and what the map can show (M29).
    pub kinds_present: Vec<String>,
    /// Number of layers.
    pub layer_count: u32,
    /// Number of objects across all layers.
    pub object_count: u32,
    /// Document revision, bumped on every change.
    ///
    /// Tile URLs carry it, so an edit makes previously fetched tiles
    /// unreachable rather than stale.
    pub revision: u64,
    /// What an image layer's picture is addressed by (spec.md 4.9, M37).
    ///
    /// Fixed for the opening, where the revision is not: a picture depends on
    /// its file, not on the document, so an edit must not re-address it. See
    /// `OpenProject::image_token`.
    pub image_token: u64,
    /// Whether there is anything to undo.
    pub can_undo: bool,
    /// Whether there is anything to redo.
    pub can_redo: bool,
}

impl ProjectSummary {
    pub(crate) fn of(open: &OpenProject) -> Self {
        let settings = open.project.settings.clone();
        Self {
            name: open.project.name.clone(),
            path: open.path.as_ref().map(|p| p.to_string_lossy().into_owned()),
            dirty: open.dirty,
            field_kind: match settings.field_kind {
                FieldKind::Wind => "wind",
                FieldKind::Current => "current",
            }
            .to_owned(),
            resolution_deg: settings.resolution.degrees(),
            resolution_label: settings.resolution.label().to_owned(),
            grid_ni: settings.resolution.ni(),
            grid_nj: settings.resolution.nj(),
            step_hours: settings.step_hours.hours(),
            step_count: settings.step_count,
            start_unix_s: settings.start_unix_s,
            direction_convention: match settings.direction_convention {
                DirectionConvention::From => "from",
                DirectionConvention::Toward => "toward",
            }
            .to_owned(),
            wind_scale_knots: settings.scale().wind_knots,
            current_scale_knots: settings.scale().current_knots,
            wind_gradient: settings.gradients().wind,
            current_gradient: settings.gradients().current,
            kinds_present: open
                .project
                .kinds_present()
                .into_iter()
                .map(|kind| kind_name(kind).to_owned())
                .collect(),
            layer_count: open.project.layers.len() as u32,
            object_count: open.project.object_count() as u32,
            revision: open.revision,
            image_token: open.image_token,
            can_undo: open.history.can_undo(),
            can_redo: open.history.can_redo(),
        }
    }
}

/// A previously opened project.
#[derive(Debug, Clone, Serialize, JsonSchema, TS)]
#[ts(export, export_to = "RecentProject.ts")]
pub struct RecentProject {
    /// Full path.
    pub path: String,
    /// File stem, for display.
    pub name: String,
    /// Whether the file is still there.
    pub exists: bool,
}

/// Settings for a project being created.
#[derive(Debug, Clone, Deserialize, TS)]
#[ts(export, export_to = "NewProjectRequest.ts")]
pub struct NewProjectRequest {
    /// Display name.
    pub name: String,
    /// The kind every layer starts as (M29): `"wind"` unless said otherwise.
    /// The dialog no longer asks — a layer says which field it is part of —
    /// so this is only the default a new layer takes.
    #[serde(default = "default_field_kind")]
    pub field_kind: String,
    /// One of `"1.0"`, `"0.5"`, `"0.25"`, `"0.1"`.
    pub resolution: String,
    /// One of 1, 3, 6, 24.
    pub step_hours: u32,
    /// Number of time steps.
    pub step_count: u32,
}

fn default_field_kind() -> String {
    "wind".to_owned()
}

/// The name a kind of field goes by across IPC.
pub fn kind_name(kind: FieldKind) -> &'static str {
    match kind {
        FieldKind::Wind => "wind",
        FieldKind::Current => "current",
    }
}

/// A kind of field from its IPC name.
pub fn parse_field_kind(value: &str) -> Result<FieldKind> {
    match value {
        "wind" => Ok(FieldKind::Wind),
        "current" => Ok(FieldKind::Current),
        other => Err(AppError::BadOption {
            field: "field_kind",
            value: other.to_owned(),
        }),
    }
}

fn parse_resolution(value: &str) -> Result<Resolution> {
    match value {
        "1.0" | "1" => Ok(Resolution::Deg1),
        "0.5" => Ok(Resolution::Deg05),
        "0.25" => Ok(Resolution::Deg025),
        "0.1" => Ok(Resolution::Deg01),
        other => Err(AppError::BadOption {
            field: "resolution",
            value: other.to_owned(),
        }),
    }
}

fn parse_step_hours(value: u32) -> Result<StepHours> {
    match value {
        1 => Ok(StepHours::H1),
        3 => Ok(StepHours::H3),
        6 => Ok(StepHours::H6),
        24 => Ok(StepHours::H24),
        other => Err(AppError::BadOption {
            field: "step_hours",
            value: other.to_string(),
        }),
    }
}

impl NewProjectRequest {
    /// Validates the request and builds the project settings.
    pub fn into_settings(&self) -> Result<ProjectSettings> {
        Ok(ProjectSettings::new(
            parse_field_kind(&self.field_kind)?,
            parse_resolution(&self.resolution)?,
            parse_step_hours(self.step_hours)?,
            self.step_count,
        ))
    }
}

pub(crate) fn with_session<T>(
    state: &AppState,
    f: impl FnOnce(&mut Session) -> Result<T>,
) -> Result<T> {
    let mut session = state
        .session
        .lock()
        .map_err(|_| AppError::Internal("session lock was poisoned".to_owned()))?;
    f(&mut session)
}

/// Creates a project and makes it the open one.
#[tauri::command]
pub fn new_project(
    state: tauri::State<'_, AppState>,
    request: NewProjectRequest,
    discard_unsaved: bool,
) -> Result<ProjectSummary> {
    create(&state, request, discard_unsaved)
}

/// Refuses to replace an open project that has unsaved changes.
///
/// Both `create` and `open` overwrite whatever is open, so both take the user's
/// answer as an argument rather than inferring it. `discard_unsaved` is that
/// answer, and nothing is dropped until the replacing call itself runs: a user
/// who says "don't save" and then cancels the file dialog still has their
/// project. The check belongs here rather than only in the dialog because the
/// result of forgetting to ask is silent, unrecoverable data loss.
pub(crate) fn refuse_to_discard(session: &Session, discard_unsaved: bool) -> Result<()> {
    match &session.open {
        Some(open) if open.dirty && !discard_unsaved => Err(AppError::UnsavedChanges {
            name: open.project.name.clone(),
        }),
        _ => Ok(()),
    }
}

/// Implementation of [`new_project`], callable without a Tauri handle.
pub fn create(
    state: &AppState,
    request: NewProjectRequest,
    discard_unsaved: bool,
) -> Result<ProjectSummary> {
    let mut settings = request.into_settings()?;
    let name = if request.name.trim().is_empty() {
        "Untitled".to_owned()
    } else {
        request.name.trim().to_owned()
    };

    with_session(state, |session| {
        refuse_to_discard(session, discard_unsaved)?;
        if let Some(replaced) = &session.open {
            crate::autosave::forget(state, replaced.project.id.raw());
        }
        // A new project takes the colour scale the user prefers for its kind
        // (spec.md 8.6, M15). The scale then belongs to the project: changing
        // the preference later leaves existing projects alone.
        settings.colour_scale = Some(
            ve_core::project::ColourScale {
                wind_knots: session.settings.default_wind_scale_knots,
                current_knots: session.settings.default_current_scale_knots,
            }
            .clamped(),
        );
        let project = Project::new(name, settings);
        tracing::info!(
            name = %project.name,
            resolution = %project.settings.resolution.label(),
            steps = project.settings.step_count,
            "created project"
        );
        session.open = Some(OpenProject::created(project));
        let open = session.require_open()?;
        Ok(ProjectSummary::of(open))
    })
}

/// Opens a project from disk.
#[tauri::command(async)]
pub fn open_project(
    state: tauri::State<'_, AppState>,
    path: String,
    discard_unsaved: bool,
) -> Result<ProjectSummary> {
    open(&state, path, discard_unsaved)
}

/// Implementation of [`open_project`], callable without a Tauri handle.
pub fn open(state: &AppState, path: String, discard_unsaved: bool) -> Result<ProjectSummary> {
    let path = PathBuf::from(path);
    let mut project = io::load(&path).doing("open the project at", path.display())?;
    tracing::info!(path = %path.display(), objects = project.object_count(), "opened project");
    // Imported fields are read back from their files, never from the project
    // (invariant 2). A file that has gone leaves its layer empty rather than
    // refusing the project; the layer panel says so.
    crate::import::attach_rasters(&mut project);

    let settings_file = state.paths.settings_file();
    with_session(state, |session| {
        refuse_to_discard(session, discard_unsaved)?;
        if let Some(replaced) = &session.open {
            crate::autosave::forget(state, replaced.project.id.raw());
        }
        session.open = Some(OpenProject::loaded(project, path.clone()));
        session.remember(&path);
        // A failure to persist the recent list must not fail the open itself.
        if let Err(err) = session.save_settings(&settings_file) {
            tracing::warn!(%err, "could not write the recent list");
        }
        let open = session.require_open()?;
        Ok(ProjectSummary::of(open))
    })
}

/// Saves the open project to its existing path.
#[tauri::command(async)]
pub fn save_project(state: tauri::State<'_, AppState>) -> Result<ProjectSummary> {
    save(&state)
}

/// Implementation of [`save_project`], callable without a Tauri handle.
pub fn save(state: &AppState) -> Result<ProjectSummary> {
    let settings_file = state.paths.settings_file();
    with_session(state, |session| {
        let path = session
            .require_open()?
            .path
            .clone()
            .ok_or(AppError::ProjectNeverSaved)?;
        session.save_to(path)?;
        if let Err(err) = session.save_settings(&settings_file) {
            tracing::warn!(%err, "could not write the recent list");
        }
        let open = session.require_open()?;
        // Saved cleanly: there is nothing to recover (spec.md 4.2, M10).
        crate::autosave::forget(state, open.project.id.raw());
        Ok(ProjectSummary::of(open))
    })
}

/// Saves the open project to a new path.
#[tauri::command(async)]
pub fn save_project_as(state: tauri::State<'_, AppState>, path: String) -> Result<ProjectSummary> {
    save_as(&state, path)
}

/// Implementation of [`save_project_as`], callable without a Tauri handle.
pub fn save_as(state: &AppState, path: String) -> Result<ProjectSummary> {
    // Add the extension if the user did not, so a project is always openable
    // by the same filter that saved it.
    let mut path = PathBuf::from(path);
    if path.extension().is_none() {
        path.set_extension(io::EXTENSION);
    }

    let settings_file = state.paths.settings_file();
    with_session(state, |session| {
        session.save_to(path.clone())?;
        if let Err(err) = session.save_settings(&settings_file) {
            tracing::warn!(%err, "could not write the recent list");
        }
        tracing::info!(path = %path.display(), "saved project");
        let open = session.require_open()?;
        crate::autosave::forget(state, open.project.id.raw());
        Ok(ProjectSummary::of(open))
    })
}

/// Closes the open project without saving.
#[tauri::command]
pub fn close_project(state: tauri::State<'_, AppState>, discard_unsaved: bool) -> Result<()> {
    close_open(&state, discard_unsaved)
}

/// Implementation of [`close_project`], callable without a Tauri handle.
///
/// Guarded like every other path that replaces the open project: the backend
/// is what knows whether the document is dirty, so it is what refuses. The
/// frontend asks first and passes the answer, exactly as `new` and `open` do —
/// and a frontend that forgot to ask still cannot drop unsaved work.
pub fn close_open(state: &AppState, discard_unsaved: bool) -> Result<()> {
    with_session(state, |session| {
        refuse_to_discard(session, discard_unsaved)?;
        // What the user chose to put down is not offered back (spec.md 4.2).
        if let Some(open) = &session.open {
            crate::autosave::forget(state, open.project.id.raw());
        }
        session.open = None;
        Ok(())
    })
}

/// The open project, or `None`.
#[tauri::command]
pub fn current_project(state: tauri::State<'_, AppState>) -> Result<Option<ProjectSummary>> {
    current(&state)
}

/// Implementation of [`current_project`], callable without a Tauri handle.
pub fn current(state: &AppState) -> Result<Option<ProjectSummary>> {
    with_session(state, |session| {
        Ok(session.open.as_ref().map(ProjectSummary::of))
    })
}

/// The recent-files list, newest first.
#[tauri::command]
pub fn recent_projects(state: tauri::State<'_, AppState>) -> Result<Vec<RecentProject>> {
    recent(&state)
}

/// Implementation of [`recent_projects`], callable without a Tauri handle.
pub fn recent(state: &AppState) -> Result<Vec<RecentProject>> {
    with_session(state, |session| {
        Ok(session
            .recent
            .iter()
            .filter(|path| path.is_file())
            .map(|path| RecentProject {
                path: path.to_string_lossy().into_owned(),
                name: path
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "untitled".to_owned()),
                exists: true,
            })
            .collect())
    })
}

/// Forgets every recent project, returning the emptied list.
///
/// Unlike the `remember` on an open, a failed write is **not** swallowed here:
/// reaching the settings file is the whole of this operation, and a list that
/// silently returns at the next launch is worse than an error now.
#[tauri::command]
pub fn clear_recent_projects(state: tauri::State<'_, AppState>) -> Result<Vec<RecentProject>> {
    clear_recent(&state)
}

/// Implementation of [`clear_recent_projects`], callable without a Tauri handle.
pub fn clear_recent(state: &AppState) -> Result<Vec<RecentProject>> {
    let settings_file = state.paths.settings_file();
    with_session(state, |session| {
        session.forget_recent();
        session.save_settings(&settings_file)
    })?;
    recent(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> NewProjectRequest {
        NewProjectRequest {
            name: "Test".to_owned(),
            field_kind: "wind".to_owned(),
            resolution: "0.25".to_owned(),
            step_hours: 3,
            step_count: 24,
        }
    }

    #[test]
    fn a_valid_request_builds_settings() {
        let settings = request().into_settings().expect("valid");
        assert_eq!(settings.field_kind, FieldKind::Wind);
        assert_eq!(settings.resolution, Resolution::Deg025);
        assert_eq!(settings.step_hours.hours(), 3);
        assert_eq!(settings.step_count, 24);
    }

    #[test]
    fn every_resolution_and_step_size_is_accepted() {
        for (value, expected) in [
            ("1.0", Resolution::Deg1),
            ("0.5", Resolution::Deg05),
            ("0.25", Resolution::Deg025),
            ("0.1", Resolution::Deg01),
        ] {
            assert_eq!(parse_resolution(value).expect(value), expected);
        }
        for hours in [1u32, 3, 6, 24] {
            assert_eq!(parse_step_hours(hours).expect("valid").hours(), hours);
        }
    }

    #[test]
    fn bad_options_are_rejected_by_name() {
        assert!(matches!(
            parse_field_kind("temperature"),
            Err(AppError::BadOption {
                field: "field_kind",
                ..
            })
        ));
        assert!(matches!(
            parse_resolution("0.3"),
            Err(AppError::BadOption {
                field: "resolution",
                ..
            })
        ));
        assert!(matches!(
            parse_step_hours(5),
            Err(AppError::BadOption {
                field: "step_hours",
                ..
            })
        ));
    }

    /// The direction convention follows the field kind, so a current project
    /// never opens showing wind's "from" convention.
    #[test]
    fn the_summary_reports_the_convention_for_the_field_kind() {
        let wind =
            OpenProject::created(Project::new("w", request().into_settings().expect("valid")));
        assert_eq!(ProjectSummary::of(&wind).direction_convention, "from");

        let mut current_request = request();
        current_request.field_kind = "current".to_owned();
        let current = OpenProject::created(Project::new(
            "c",
            current_request.into_settings().expect("valid"),
        ));
        let summary = ProjectSummary::of(&current);
        assert_eq!(summary.direction_convention, "toward");
        assert_eq!(summary.field_kind, "current");
    }

    #[test]
    fn the_summary_carries_the_grid_dimensions() {
        let open =
            OpenProject::created(Project::new("g", request().into_settings().expect("valid")));
        let summary = ProjectSummary::of(&open);
        assert_eq!((summary.grid_ni, summary.grid_nj), (1440, 721));
        assert_eq!(summary.resolution_label, "0.25°");
        assert!(
            summary.dirty,
            "a new project has unsaved changes by definition"
        );
        assert!(summary.path.is_none());
    }

    #[test]
    fn step_count_is_clamped_rather_than_rejected() {
        let mut oversized = request();
        oversized.step_count = 100_000;
        assert_eq!(oversized.into_settings().expect("valid").step_count, 240);
    }
}

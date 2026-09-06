//! Importing a GRIB2 file as layers (spec.md 4.8).
//!
//! The file is decoded outside the session lock — a global 0.25° file is
//! seconds of work — and then added to the document as one layer per field
//! kind it holds, in a single history entry. The document keeps the file's
//! path and nothing else; the decoded field lives beside the layer in memory
//! and is read again from the path when the project is next opened
//! (invariants 1 and 2).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use ve_core::Command;
use ve_core::document::{Layer, LayerSource};
use ve_core::project::{FieldKind, MAX_STEPS, Project, ProjectSettings, Resolution, StepHours};
use ve_core::raster::RasterSequence;
use ve_grib::import;

use crate::commands::AppState;
use crate::error::{AppError, Result};
use crate::projects::{ProjectSummary, with_session};
use crate::session::OpenProject;

/// The file's own name, for layer names and history labels.
fn layer_name_suffix(path: &Path) -> String {
    path.file_name()
        .map(|f| f.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

/// The layer name for an imported field: what it is, and where from.
fn layer_name(kind: FieldKind, path: &Path) -> String {
    let what = match kind {
        FieldKind::Wind => "Wind",
        FieldKind::Current => "Currents",
    };
    format!("{what} ({})", layer_name_suffix(path))
}

/// Imports a GRIB2 file as one or more layers above the current top.
#[tauri::command(async)]
pub fn import_grib(state: tauri::State<'_, AppState>, path: String) -> Result<ProjectSummary> {
    grib_import(&state, path)
}

/// Implementation of [`import_grib`].
///
/// A file holding both wind and currents becomes two layers. The one whose
/// kind matches the project is shown; the other is imported hidden, since
/// the map composites every visible layer into one field and a current drawn
/// over a wind is not a wind (spec.md 4.8).
pub fn grib_import(state: &AppState, path: String) -> Result<ProjectSummary> {
    let path = PathBuf::from(path);
    with_session(state, |session| {
        let open = session.require_open()?;
        // An unstructured file is resampled onto *this* project's grid, so
        // the read cannot happen before the project is in hand.
        let imported = resample_into(&mut open.project, &path)?;
        for skipped in &imported.skipped {
            tracing::warn!(
                path = %path.display(),
                message = skipped.index + 1,
                reason = %skipped.reason,
                "grib message skipped on import"
            );
        }
        let open = session.require_open()?;
        add_layers(open, &path, imported.sequences)?;
        Ok(ProjectSummary::of(session.require_open()?))
    })
}

/// Reads a file for an open project, resampling onto that project's grid and
/// keeping whatever neighbour sets the resample had to compute.
fn resample_into(project: &mut Project, path: &Path) -> Result<import::Imported> {
    let target = project.settings.resolution.target_grid();
    let mut cache = std::mem::take(&mut project.regrid);
    let result = {
        let mut resampling = import::Resampling::new(target, &mut cache);
        import::read_file(path, Some(&mut resampling))
    };
    project.regrid = cache;
    Ok(result?)
}

/// Adds one layer per imported field above the open project's top, as one
/// history entry.
fn add_layers(open: &mut OpenProject, path: &Path, sequences: Vec<RasterSequence>) -> Result<()> {
    let mut commands = Vec::with_capacity(sequences.len());
    for sequence in sequences {
        let kind = sequence.kind;
        tracing::info!(
            path = %path.display(),
            ?kind,
            frames = sequence.frames.len(),
            span_hours = sequence.span_hours(),
            "imported grib field"
        );
        let layer = Layer::from_grib(
            layer_name(kind, path),
            path.to_path_buf(),
            Arc::new(sequence),
            // Shown whatever its kind (M29): the map shows one kind at a
            // time, so a current file in a wind project is a layer the map
            // turns to, not one to hide.
            true,
        );
        // Each layer goes on top of the last: the index is where it will
        // land once the ones before it in the batch have been added.
        commands.push(Command::AddLayer {
            index: open.project.layers.len() + commands.len(),
            layer: Box::new(layer),
        });
    }
    let command = if commands.len() == 1 {
        commands.remove(0)
    } else {
        Command::Batch {
            label: format!("Import {}", layer_name_suffix(path)),
            commands,
        }
    };
    let (project, history) = (&mut open.project, &mut open.history);
    history.push(project, command)?;
    open.touch();
    Ok(())
}

/// The app resolution nearest a file's own spacing.
///
/// Shared with the unstructured path, which has to choose the grid *before*
/// resampling onto it — so the choice cannot live inside the code that reads
/// the resampled result (spec.md 4.8).
pub fn nearest_resolution(spacing: f64) -> Resolution {
    Resolution::ALL
        .iter()
        .copied()
        .min_by(|a, b| {
            let da = (a.degrees() - spacing).abs();
            let db = (b.degrees() - spacing).abs();
            da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
        })
        .unwrap_or(Resolution::Deg1)
}

/// Project settings that fit an imported file (spec.md 4.8).
///
/// The kind is wind when the file has wind, current otherwise; the grid is
/// the app resolution nearest the file's spacing; the step is the largest the
/// app offers that lands on *every* message, so none of them is invisible; the
/// count covers the file's span; and step 0 is stamped with the first
/// message's valid time.
pub fn settings_for(sequences: &[RasterSequence]) -> Option<ProjectSettings> {
    let sequence = sequences
        .iter()
        .find(|s| s.kind == FieldKind::Wind)
        .or_else(|| sequences.first())?;
    let first = sequence.frames.first()?;

    let resolution = nearest_resolution(first.grid.dlon.min(first.grid.dlat));

    // The largest offered step that every message lands on. A step is shown
    // from the file only where the file has a message for that exact time
    // (spec.md 4.8), so a step size that does not divide the message times is
    // one that hides most of the file: a 4-hourly file on 3-hourly steps would
    // show its 0 h and 12 h messages and none of the rest. Divisibility is the
    // property that matters, not merely being no larger than the gap.
    let divides_every_message = |step: StepHours| {
        let hours = f64::from(step.hours());
        sequence.frames.iter().all(|frame| {
            let steps = frame.offset_hours / hours;
            (steps - steps.round()).abs() < 1e-6
        })
    };
    let step_hours = if sequence.frames.len() < 2 {
        // One message lands on step 0 whatever the step size, so the size is a
        // decision about the timeline the user is about to build rather than
        // about the file. The middle of the range is the kindest default.
        StepHours::H3
    } else {
        [StepHours::H24, StepHours::H6, StepHours::H3, StepHours::H1]
            .into_iter()
            .find(|step| divides_every_message(*step))
            // Nothing divides them — a file on a 90-minute cadence, say. The finest
            // step the app has shows the most of it.
            .unwrap_or(StepHours::H1)
    };

    let steps = (sequence.span_hours() / f64::from(step_hours.hours())).floor() as u32 + 1;
    let mut settings = ProjectSettings::new(
        sequence.kind,
        resolution,
        step_hours,
        steps.clamp(1, MAX_STEPS),
    );
    settings.start_unix_s = Some(first.valid_unix_s);
    Some(settings)
}

/// Creates a project shaped by a GRIB2 file and imports the file into it.
#[tauri::command(async)]
pub fn new_project_from_grib(
    state: tauri::State<'_, AppState>,
    path: String,
    discard_unsaved: bool,
) -> Result<ProjectSummary> {
    grib_project(&state, path, discard_unsaved)
}

/// Implementation of [`new_project_from_grib`].
///
/// The project takes its name from the file and its settings from
/// [`settings_for`]; the file then imports as it would into any project, so
/// the layers are the same ones **Import GRIB** would have made. Replacing an
/// unsaved project is refused unless `discard_unsaved` says otherwise, as
/// with every other command that replaces one.
pub fn grib_project(
    state: &AppState,
    path: String,
    discard_unsaved: bool,
) -> Result<ProjectSummary> {
    let path = PathBuf::from(path);
    // The grid comes first: an unstructured file states no spacing, so the
    // resolution is derived from its mesh and the file is then resampled onto
    // it. A lat/lon file reaches the same answer from its own increments.
    let (messages, skipped) = import::read_messages(&path)?;
    let resolution = nearest_resolution(import::nominal_spacing(&messages).unwrap_or(1.0));
    let mut cache = std::collections::BTreeMap::new();
    let sequences = {
        let mut resampling = import::Resampling::new(resolution.target_grid(), &mut cache);
        import::sequences(messages, Some(&mut resampling))?
    };
    let imported = import::Imported { sequences, skipped };
    let settings = settings_for(&imported.sequences).ok_or_else(|| {
        AppError::Grib(ve_grib::GribError::NoVectorField(
            "the file holds no time step to build a project from".to_owned(),
        ))
    })?;
    let name = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "Untitled".to_owned());

    with_session(state, |session| {
        crate::projects::refuse_to_discard(session, discard_unsaved)?;
        let project = Project::new(name, settings);
        tracing::info!(
            name = %project.name,
            path = %path.display(),
            resolution = %settings.resolution.label(),
            step_hours = settings.step_hours.hours(),
            steps = settings.step_count,
            "created project from grib"
        );
        session.open = Some(OpenProject::created(project));
        let open = session.require_open()?;
        add_layers(open, &path, imported.sequences)?;
        // The import is what the project is, not an edit to it: the history
        // starts empty, as it does for a project just created.
        open.history = Default::default();
        Ok(ProjectSummary::of(session.require_open()?))
    })
}

/// Reads every GRIB layer's file back into memory after a project opens.
///
/// A file that cannot be read leaves its layer without a field — it still
/// opens, still lists, and contributes nothing — and the reason is logged
/// and returned so the caller can show it. The project itself is never
/// refused over a missing import: the user's own work is in the objects,
/// and those are intact.
pub fn attach_rasters(project: &mut Project) -> Vec<(String, AppError)> {
    let mut failures = Vec::new();
    // The neighbour sets travel with the project, so a reopened ICON layer
    // costs a decode and an interpolation rather than the search as well.
    // Taken out for the loop's sake and put back after, sets and all.
    let resolution = project.settings.resolution;
    let mut cache = std::mem::take(&mut project.regrid);
    for layer in &mut project.layers {
        let LayerSource::Grib { path, field } = &layer.source else {
            continue;
        };
        let (path, field) = (path.clone(), *field);
        let target = resolution.target_grid();
        let result = {
            let mut resampling = import::Resampling::new(target, &mut cache);
            import::read_file(&path, Some(&mut resampling))
        };
        match result {
            Ok(imported) => {
                layer.raster = imported
                    .sequences
                    .into_iter()
                    .find(|s| s.kind == field)
                    .map(Arc::new);
                if layer.raster.is_none() {
                    let err = AppError::Grib(ve_grib::GribError::NoVectorField(format!(
                        "{} no longer holds a {field:?} field",
                        path.display()
                    )));
                    tracing::warn!(layer = %layer.name, %err, "grib layer has no field");
                    failures.push((layer.name.clone(), err));
                }
            }
            Err(err) => {
                layer.raster = None;
                let err = AppError::from(err);
                tracing::warn!(layer = %layer.name, path = %path.display(), %err, "grib layer could not be read");
                failures.push((layer.name.clone(), err));
            }
        }
    }
    project.regrid = cache;
    failures
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ve_core::raster::{RasterFrame, RasterGrid, RasterSequence};

    use super::*;

    fn sequence(kind: FieldKind, spacing: f64, offsets: &[f64]) -> RasterSequence {
        let ni = (360.0 / spacing).round() as u32;
        let nj = (180.0 / spacing).round() as u32 + 1;
        let frames = offsets
            .iter()
            .map(|&h| RasterFrame {
                offset_hours: h,
                valid_unix_s: 1_000_000 + (h * 3600.0) as i64,
                grid: Arc::new(
                    RasterGrid::new(
                        ni,
                        nj,
                        0.0,
                        90.0,
                        spacing,
                        spacing,
                        vec![[0.0, 0.0]; (ni * nj) as usize],
                    )
                    .expect("grid"),
                ),
            })
            .collect();
        RasterSequence::new(kind, frames).expect("sequence")
    }

    #[test]
    fn the_grid_is_the_nearest_resolution() {
        for (spacing, expected) in [
            (1.0, Resolution::Deg1),
            (0.5, Resolution::Deg05),
            (0.25, Resolution::Deg025),
            (0.1, Resolution::Deg01),
            // Off the ladder: 0.125 is nearer 0.1, 0.2 nearer 0.25, 2 nearer 1.
            (0.125, Resolution::Deg01),
            (0.2, Resolution::Deg025),
            (2.0, Resolution::Deg1),
        ] {
            let settings =
                settings_for(&[sequence(FieldKind::Wind, spacing, &[0.0])]).expect("some");
            assert_eq!(settings.resolution, expected, "spacing {spacing}");
        }
    }

    #[test]
    fn the_step_lands_on_every_message_the_file_has() {
        let hours = |offsets: &[f64]| {
            settings_for(&[sequence(FieldKind::Wind, 1.0, offsets)])
                .expect("some")
                .step_hours
                .hours()
        };
        assert_eq!(hours(&[0.0, 1.0, 2.0]), 1);
        assert_eq!(hours(&[0.0, 3.0, 6.0]), 3);
        assert_eq!(hours(&[0.0, 6.0]), 6);
        assert_eq!(
            hours(&[0.0, 12.0]),
            6,
            "12 h is not offered; 6 h lands on both messages"
        );
        assert_eq!(hours(&[0.0, 24.0]), 24);
        assert_eq!(
            hours(&[0.0, 2.0]),
            1,
            "2 h is not offered; hourly lands on both, and blanks the odd hours"
        );
        // Mixed spacing: what divides them all, not the smallest gap. A
        // 4-hourly file would take hourly steps rather than 3-hourly ones,
        // which would land on one message in four.
        assert_eq!(hours(&[0.0, 3.0, 6.0, 12.0, 24.0]), 3);
        assert_eq!(hours(&[0.0, 4.0, 8.0, 12.0]), 1);
        assert_eq!(hours(&[0.0]), 3, "one message: the default step");
    }

    #[test]
    fn the_count_covers_the_span_and_the_start_is_the_first_message() {
        let settings = settings_for(&[sequence(FieldKind::Current, 0.5, &[0.0, 3.0, 6.0, 9.0])])
            .expect("some");
        assert_eq!(settings.step_count, 4);
        assert_eq!(settings.start_unix_s, Some(1_000_000));
        assert_eq!(settings.field_kind, FieldKind::Current);
        // A span past the app's ceiling is clamped.
        let long: Vec<f64> = (0..300).map(f64::from).collect();
        let settings = settings_for(&[sequence(FieldKind::Wind, 1.0, &long)]).expect("some");
        assert_eq!(settings.step_count, MAX_STEPS);
    }

    #[test]
    fn wind_wins_when_a_file_has_both() {
        let both = [
            sequence(FieldKind::Wind, 0.25, &[0.0, 3.0]),
            sequence(FieldKind::Current, 1.0, &[0.0, 24.0]),
        ];
        let settings = settings_for(&both).expect("some");
        assert_eq!(settings.field_kind, FieldKind::Wind);
        assert_eq!(settings.resolution, Resolution::Deg025);
        assert_eq!(settings.step_hours.hours(), 3);
        assert!(settings_for(&[]).is_none());
    }
}

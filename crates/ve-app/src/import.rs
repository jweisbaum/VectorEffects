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
use ve_core::document::Layer;
use ve_core::project::{FieldKind, MAX_STEPS, Project, ProjectSettings, Resolution, StepHours};
use ve_core::raster::RasterSequence;
use ve_grib::import;

use crate::commands::AppState;
use crate::error::{AppError, Context, Result};
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
        add_layers(open, &path, imported.sequences, false)?;
        Ok(ProjectSummary::of(session.require_open()?))
    })
}

/// Reads a file for an open project, resampling onto that project's grid and
/// keeping whatever neighbour sets the resample had to compute.
fn resample_into(project: &mut Project, path: &Path) -> Result<import::Imported> {
    let mut cache = std::mem::take(&mut project.regrid);
    let result = {
        let mut resampling = resampling_for(&project.settings, &mut cache);
        import::read_file(path, Some(&mut resampling))
    };
    project.regrid = cache;
    result.map_err(|err| grib_error(err, path, &project.settings, "import the GRIB file at"))
}

/// What a project's files are read onto (spec.md 4.8, M101).
///
/// A global project resamples onto its resolution's grid and keeps every
/// lat/lon file whole, exactly as it always has, so its lattices, their
/// hashes and the render cache are untouched. A regional one reads every
/// file onto the region's lattice, cropped to it plus a node.
pub(crate) fn resampling_for<'a>(
    settings: &ProjectSettings,
    cache: &'a mut std::collections::BTreeMap<String, Arc<ve_core::regrid::Neighbours>>,
) -> import::Resampling<'a> {
    match settings.region {
        Some(_) => import::Resampling::regional(settings.lattice(), cache),
        None => import::Resampling::new(settings.resolution.target_grid(), cache),
    }
}

/// The lattice a regional project crops its imports to; `None` when global.
pub(crate) fn crop_for(settings: &ProjectSettings) -> Option<ve_core::regrid::TargetGrid> {
    settings.region.map(|_| settings.lattice())
}

/// A GRIB reader's failure, said in the app's words: a file outside the
/// region is refused naming both (decision R10); anything else names what
/// was being done and to which file.
fn grib_error(
    err: ve_grib::GribError,
    path: &Path,
    settings: &ProjectSettings,
    doing: &'static str,
) -> AppError {
    match (err, settings.region) {
        (ve_grib::GribError::OutsideRegion, Some(region)) => {
            AppError::outside_region(path.display(), &region)
        }
        (err, _) => AppError::Doing {
            doing,
            what: path.display().to_string(),
            why: err.to_string(),
        },
    }
}

/// Adds one layer per imported field above the open project's top, as one
/// history entry.
pub(crate) fn add_layers(
    open: &mut OpenProject,
    path: &Path,
    sequences: Vec<RasterSequence>,
    zarr: bool,
) -> Result<()> {
    let mut commands = Vec::with_capacity(sequences.len());
    for sequence in sequences {
        let kind = sequence.kind;
        tracing::info!(
            path = %path.display(),
            ?kind,
            frames = sequence.frames.len(),
            span_hours = sequence.span_hours(),
            "imported raster field"
        );
        let mut layer = Layer::from_grib(
            layer_name(kind, path),
            path.to_path_buf(),
            Arc::new(sequence),
            // Shown whatever its kind (M29): the map shows one kind at a
            // time, so a current file in a wind project is a layer the map
            // turns to, not one to hide.
            true,
        );
        if zarr {
            layer.source = ve_core::document::LayerSource::ZarrFile {
                path: path.to_path_buf(),
                field: kind,
            };
        }
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
    settings.region = region_of_lattice(&first.grid, resolution);
    Some(settings)
}

/// The region a file's lattice asks for, or `None` for a global project
/// (decision R9).
///
/// A file whose lattice is global gives a global project. Any other gives its
/// own extent snapped **outward** at `resolution`, so no node of the file is
/// cut off. A lattice that wraps but misses a pole is a full-circle band; and
/// an extent that snaps to the whole earth is global too.
pub(crate) fn region_of_lattice(
    grid: &ve_core::raster::RasterGrid,
    resolution: Resolution,
) -> Option<ve_core::region::Region> {
    let south = grid.lat0 - f64::from(grid.nj - 1) * grid.dlat;
    let north = grid.lat0;
    // A lattice one row short of a pole still reaches it: the routing
    // stores leave the south pole row out (ve-zarr's `Layout`).
    let both_poles = north >= 90.0 - grid.dlat / 2.0 && south <= -90.0 + grid.dlat * 1.5;
    if grid.wraps && both_poles {
        return None;
    }
    let east = grid.lon0 + f64::from(grid.ni - 1) * grid.dlon;
    // A refusal here is the whole earth or a degenerate box: the project is
    // then global, which holds every node of the file.
    ve_core::region::Region::snapped(
        grid.lon0,
        east,
        south.max(-90.0),
        north.min(90.0),
        grid.wraps,
        resolution,
    )
    .ok()
}

/// The region a *resampled* lattice asks for: the extent of its nodes that
/// hold a value, or `None` for a global project (decision R9).
///
/// A projected or unstructured file has no lattice of its own and is put on
/// the global one, where everything its grid does not reach is missing. Its
/// extent is therefore what is present: latitude from the first and last rows
/// holding any value, longitude the whole circle less its largest run of
/// columns holding none (so a field across 180 degrees is one arc, not a
/// band round the world). No gap means a full circle. Snapped outward.
pub(crate) fn region_of_data(
    grid: &ve_core::raster::RasterGrid,
    resolution: Resolution,
) -> Option<ve_core::region::Region> {
    if !grid.wraps {
        // A patch has its own edges; absence of a gap says nothing.
        return region_of_lattice(grid, resolution);
    }
    let (ni, nj) = (grid.ni as usize, grid.nj as usize);
    let mut row_has = vec![false; nj];
    let mut col_has = vec![false; ni];
    for (j, row) in grid.uv.chunks(ni).enumerate() {
        for (i, [u, v]) in row.iter().enumerate() {
            if !ve_core::raster::is_missing(*u) && !ve_core::raster::is_missing(*v) {
                row_has[j] = true;
                col_has[i] = true;
            }
        }
    }
    let first_row = row_has.iter().position(|&h| h)?;
    let last_row = row_has.iter().rposition(|&h| h)?;
    let north = grid.lat0 - first_row as f64 * grid.dlat;
    let south = grid.lat0 - last_row as f64 * grid.dlat;
    // The largest circular run of empty columns: its far side is the west
    // edge, its near side the east.
    let (mut best_len, mut best_start) = (0usize, 0usize);
    let mut run = 0usize;
    for k in 0..2 * ni {
        if col_has[k % ni] {
            run = 0;
        } else {
            run += 1;
            if run > best_len && run < ni {
                best_len = run;
                best_start = k + 1 - run;
            }
        }
    }
    let full_circle = best_len == 0;
    let lon_of = |i: usize| grid.lon0 + (i % ni) as f64 * grid.dlon;
    let west = lon_of(best_start + best_len);
    let east = lon_of(best_start + ni - 1);
    ve_core::region::Region::snapped(
        west,
        east,
        south.max(-90.0),
        north.min(90.0),
        full_circle,
        resolution,
    )
    .ok()
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
    with_session(state, |session| {
        crate::projects::refuse_to_discard(session, discard_unsaved)
    })?;
    // The loading page's bar, on the scale `read_file_reporting` uses: a
    // message unpacked is one unit and a frame built is two.
    let mut opening = state.opening.begin();
    opening.sources([layer_name_suffix(&path)]);
    let (messages, skipped) =
        import::read_messages_reporting(&path, &|done, total| opening.source(0, done, total * 2))
            .doing("read the GRIB file at", path.display())?;
    let resolution = nearest_resolution(import::nominal_spacing(&messages).unwrap_or(1.0));
    // A file with no lattice of its own is put on the global one, so its
    // extent is read from where it has values (decision R9).
    let resampled = messages
        .iter()
        .any(|m| !matches!(m.header.grid, ve_grib::decode::Grid::LatLon(_)));
    let mut cache = std::collections::BTreeMap::new();
    let sequences = {
        let unpacked = messages.len();
        let mut resampling = import::Resampling::new(resolution.target_grid(), &mut cache);
        import::sequences_reporting(messages, Some(&mut resampling), &|frames, _| {
            opening.source(0, (unpacked + frames * 2).min(unpacked * 2), unpacked * 2);
        })?
    };
    let mut imported = import::Imported { sequences, skipped };
    let mut settings = settings_for(&imported.sequences).ok_or_else(|| {
        AppError::Grib(ve_grib::GribError::NoVectorField(
            "the file holds no time step to build a project from".to_owned(),
        ))
    })?;
    if resampled
        && let Some(first) = imported
            .sequences
            .iter()
            .flat_map(|s| s.frames.first())
            .next()
    {
        settings.region = region_of_data(&first.grid, settings.resolution);
    }
    // A file that is not global makes a regional project (decision R9), and
    // is then read the way that project will read it on every reopening: onto
    // the region's lattice, so a save and a load hold the same rasters.
    if settings.region.is_some() {
        let mut project = Project::new("scratch".to_owned(), settings.clone());
        imported = resample_into(&mut project, &path)?;
    }
    opening.finished();
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
            resolution = %project.settings.resolution.label(),
            step_hours = project.settings.step_hours.hours(),
            steps = project.settings.step_count,
            "created project from grib"
        );
        session.open = Some(OpenProject::created(project));
        let open = session.require_open()?;
        add_layers(open, &path, imported.sequences, false)?;
        // The import is what the project is, not an edit to it: the history
        // starts empty, as it does for a project just created.
        open.history = Default::default();
        Ok(ProjectSummary::of(session.require_open()?))
    })
}

/// A reader's failure on reopening: a file that now misses the region says so
/// in the words an import would (decision R10); anything else is as it was.
fn reopen_error(err: ve_grib::GribError, path: &Path, settings: &ProjectSettings) -> AppError {
    match (err, settings.region) {
        (ve_grib::GribError::OutsideRegion, Some(region)) => {
            AppError::outside_region(path.display(), &region)
        }
        (err, _) => AppError::from(err),
    }
}

/// Reads every imported layer's file back into memory after a project opens.
///
/// A file that cannot be read leaves its layer without a field — it still
/// opens, still lists, and contributes nothing — and the reason is logged
/// and returned so the caller can show it. The project itself is never
/// refused over a missing import: the user's own work is in the objects,
/// and those are intact.
///
/// **A file is read once however many layers name it.** A GRIB holding wind
/// and currents is two layers and one decode, as a routing store always was;
/// the layers share the sequences. Different files are read side by side:
/// each is its own work, and a project built from a forecast and a history
/// import waits for the slower of the two rather than for both.
pub fn attach_rasters(
    project: &mut Project,
    opening: &mut crate::opening::Opening,
) -> Vec<(String, AppError)> {
    use rayon::prelude::*;
    use ve_core::document::LayerSource;

    // The distinct files, in the order the layers first name them, and which
    // of them each layer reads.
    let mut sources: Vec<(PathBuf, bool)> = Vec::new();
    let mut wanted: Vec<(usize, usize, FieldKind)> = Vec::new();
    for (index, layer) in project.layers.iter().enumerate() {
        // A history layer reads a GRIB file of its own (M38), so it comes
        // back the same way a forecast does.
        let Some((path, field)) = layer.source.raster_file() else {
            continue;
        };
        let source = (
            path.to_path_buf(),
            matches!(layer.source, LayerSource::ZarrFile { .. }),
        );
        let at = sources
            .iter()
            .position(|known| *known == source)
            .unwrap_or_else(|| {
                sources.push(source);
                sources.len() - 1
            });
        wanted.push((index, at, field));
    }
    opening.sources(sources.iter().map(|(path, _)| layer_name_suffix(path)));

    // The neighbour sets travel with the project, so a reopened ICON layer
    // costs a decode and an interpolation rather than the search as well.
    // Every file starts from the project's sets — they are shared, so the
    // copy is a map of pointers — and what each had to build goes back after.
    //
    // A regional project reads every file onto its region (spec.md 4.8,
    // M101), on every open: a file stays where the person put it and is
    // cropped in memory, never copied (R4).
    let settings = project.settings.clone();
    let regrid = std::mem::take(&mut project.regrid);
    let opening = &*opening;
    let read: Vec<_> = sources
        .par_iter()
        .enumerate()
        .map(|(at, (path, zarr))| {
            let progress = |done: usize, total: usize| opening.source(at, done, total);
            let mut cache = regrid.clone();
            let sequences = if *zarr {
                crate::zarr::read_for(path, &settings, &progress)
            } else {
                let mut resampling = resampling_for(&settings, &mut cache);
                import::read_file_reporting(path, Some(&mut resampling), &progress)
                    .map(|imported| imported.sequences)
                    .map_err(|err| reopen_error(err, path, &settings))
            };
            let sequences = sequences.map(|s| s.into_iter().map(Arc::new).collect::<Vec<_>>());
            (sequences, cache)
        })
        .collect();

    project.regrid = regrid;
    let mut results = Vec::with_capacity(read.len());
    for (sequences, cache) in read {
        for (key, set) in cache {
            project.regrid.entry(key).or_insert(set);
        }
        results.push(sequences);
    }

    let mut failures = Vec::new();
    // An SST layer's days come back from their own file, as temperatures
    // rather than a field (spec.md 4.10, M93).
    for layer in &mut project.layers {
        let LayerSource::Sst { path, .. } = &layer.source else {
            continue;
        };
        let crop = crop_for(&settings);
        match ve_grib::import::read_temperature_file_onto(path, crop.as_ref()) {
            Ok(sequence) => layer.temperature = Some(Arc::new(sequence)),
            Err(err) => {
                let err = reopen_error(err, path, &settings);
                layer.temperature = None;
                tracing::warn!(layer = %layer.name, path = %path.display(), %err, "SST layer could not be read");
                failures.push((layer.name.clone(), AppError::Internal(err.to_string())));
            }
        }
    }
    for (index, at, field) in wanted {
        let layer = &mut project.layers[index];
        let path = &sources[at].0;
        match &results[at] {
            Ok(sequences) => {
                layer.raster = sequences.iter().find(|s| s.kind == field).cloned();
                if layer.raster.is_none() {
                    let err = AppError::Grib(ve_grib::GribError::NoVectorField(format!(
                        "{} no longer holds a {field:?} field",
                        path.display()
                    )));
                    tracing::warn!(layer = %layer.name, %err, "imported layer has no field");
                    failures.push((layer.name.clone(), err));
                }
            }
            Err(err) => {
                layer.raster = None;
                tracing::warn!(layer = %layer.name, path = %path.display(), %err, "imported layer could not be read");
                // One file's error is every layer's that names it, and an
                // error is not something to copy: the message is.
                failures.push((layer.name.clone(), AppError::Internal(err.to_string())));
            }
        }
    }
    failures
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ve_core::raster::{RasterFrame, RasterGrid, RasterSequence};

    use super::*;

    fn lattice(ni: u32, nj: u32, lon0: f64, lat0: f64, d: f64) -> ve_core::raster::RasterGrid {
        ve_core::raster::RasterGrid::new(
            ni,
            nj,
            lon0,
            lat0,
            d,
            d,
            vec![[1.0, 0.0]; (ni * nj) as usize],
        )
        .expect("grid")
    }

    #[test]
    fn a_global_lattice_seeds_no_region() {
        let grid = lattice(360, 181, -180.0, 90.0, 1.0);
        assert!(region_of_lattice(&grid, Resolution::Deg1).is_none());
    }

    #[test]
    fn a_lattice_one_row_short_of_the_south_pole_is_still_global() {
        let grid = lattice(360, 180, -180.0, 90.0, 1.0);
        assert!(region_of_lattice(&grid, Resolution::Deg1).is_none());
    }

    #[test]
    fn a_wrapping_band_is_a_full_circle_region() {
        let grid = lattice(360, 121, -180.0, 60.0, 1.0);
        let region = region_of_lattice(&grid, Resolution::Deg1).expect("a band");
        assert!(region.is_full_circle());
        assert_eq!(region.bounds_deg(), (-180.0, 180.0, -60.0, 60.0));
    }

    #[test]
    fn a_lattice_across_the_antimeridian_keeps_its_arc() {
        // 170E to 170W: twenty degrees, 21 columns.
        let grid = lattice(21, 11, 170.0, 10.0, 1.0);
        let region = region_of_lattice(&grid, Resolution::Deg1).expect("a box");
        assert_eq!(region.bounds_deg(), (170.0, 190.0, 0.0, 10.0));
    }

    #[test]
    fn a_lattice_reaching_the_north_pole_is_a_box_to_the_pole() {
        let grid = lattice(41, 21, 10.0, 90.0, 1.0);
        let region = region_of_lattice(&grid, Resolution::Deg1).expect("a cap");
        assert_eq!(region.bounds_deg(), (10.0, 50.0, 70.0, 90.0));
    }

    #[test]
    fn an_extent_that_snaps_to_the_whole_earth_is_global() {
        // -180 to 180 on a 0.5 degree file repeats the seam column; pole to
        // pole it is the earth, not a box.
        let grid = lattice(721, 361, -180.0, 90.0, 0.5);
        assert!(region_of_lattice(&grid, Resolution::Deg05).is_none());
    }

    #[test]
    fn a_resampled_lattice_is_where_it_has_values() {
        // A 1 degree global lattice holding data only from 175E to 175W and
        // 40N to 50N, the rest missing.
        let mut grid = lattice(360, 181, -180.0, 90.0, 1.0);
        for v in &mut grid.uv {
            *v = [ve_core::raster::MISSING; 2];
        }
        for lat in 40..=50 {
            for lon in (175..180).chain(-180..=-175) {
                let j = (90 - lat) as usize;
                let i = (lon + 180) as usize;
                grid.uv[j * 360 + i] = [3.0, 4.0];
            }
        }
        let region = region_of_data(&grid, Resolution::Deg1).expect("regional");
        assert_eq!(region.bounds_deg(), (175.0, 185.0, 40.0, 50.0));
        // A field everywhere is the earth.
        let full = lattice(360, 181, -180.0, 90.0, 1.0);
        assert!(region_of_data(&full, Resolution::Deg1).is_none());
        // Nothing anywhere is not a region.
        let mut empty = lattice(360, 5, -180.0, 10.0, 1.0);
        empty
            .uv
            .iter_mut()
            .for_each(|v| *v = [ve_core::raster::MISSING; 2]);
        assert!(region_of_data(&empty, Resolution::Deg1).is_none());
    }

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

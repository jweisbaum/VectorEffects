//! Local routing Zarr imports use the same raster, history, and editing path
//! as GRIB imports. Only decoding and the persisted source differ.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use ve_core::project::{FieldKind, Project};
use ve_core::raster::{MISSING, RasterFrame, RasterGrid, RasterSequence};
use ve_zarr::routing::RoutingStore;

use crate::commands::AppState;
use crate::error::{AppError, Context, Result};
use crate::projects::{ProjectSummary, with_session};
use crate::session::OpenProject;

/// Decode into ordinary raster sequences, retaining the native lattice.
pub fn read(path: &Path) -> Result<Vec<RasterSequence>> {
    read_store(path).doing("read the Zarr directory at", path.display())
}

fn read_store(path: &Path) -> Result<Vec<RasterSequence>> {
    let store = RoutingStore::open(path).map_err(|e| AppError::Internal(e.to_string()))?;
    let mut fields = Vec::new();
    for (kind, u, v) in [
        (FieldKind::Wind, "u10", "v10"),
        (FieldKind::Current, "ucur", "vcur"),
    ] {
        if let (Some(u), Some(v)) = (
            store.parameters.iter().position(|p| p == u),
            store.parameters.iter().position(|p| p == v),
        ) {
            fields.push((kind, u, v, Vec::with_capacity(store.times.len())));
        }
    }
    let ni = store.longitude.len();
    let nj = store.latitude.len();
    let points = ni
        .checked_mul(nj)
        .ok_or_else(|| AppError::Internal("Zarr grid is too large".into()))?;
    let stride = points
        .checked_mul(store.parameters.len())
        .ok_or_else(|| AppError::Internal("Zarr grid is too large".into()))?;
    // Bound temporary f32 slabs to 128 MiB. The final immutable rasters are
    // shared by rendering and undo, just as they are for a GRIB import.
    let batch = ((128 * 1024 * 1024) / std::mem::size_of::<f32>() / stride).max(1);
    // Routing test currents are constant. Identical frames (including empty
    // ones) share their grid instead of retaining a month's duplicate data.
    let mut grids = HashMap::<[u8; 32], Arc<RasterGrid>>::new();
    for start in (0..store.times.len()).step_by(batch) {
        let end = (start + batch).min(store.times.len());
        let data = store
            .read(start..end)
            .map_err(|e| AppError::Internal(e.to_string()))?;
        for t in start..end {
            let offset = (t - start) * stride;
            for (_, u, v, frames) in &mut fields {
                let u = &data[offset + *u * points..offset + (*u + 1) * points];
                let v = &data[offset + *v * points..offset + (*v + 1) * points];
                let uv = u
                    .iter()
                    .zip(v)
                    .map(|(&u, &v)| {
                        if u.is_finite() && v.is_finite() {
                            [u, v]
                        } else {
                            [MISSING; 2]
                        }
                    })
                    .collect();
                let grid = RasterGrid::new(
                    ni as u32,
                    nj as u32,
                    store.longitude[0],
                    store.latitude[0],
                    (store.longitude[ni - 1] - store.longitude[0]) / (ni - 1) as f64,
                    (store.latitude[0] - store.latitude[nj - 1]) / (nj - 1) as f64,
                    uv,
                )
                .map_err(AppError::Internal)?;
                let grid = grids
                    .entry(grid.hash)
                    .or_insert_with(|| Arc::new(grid))
                    .clone();
                frames.push(RasterFrame {
                    offset_hours: (store.times[t] - store.times[0]) as f64 / 3600.0,
                    valid_unix_s: store.times[t],
                    grid,
                });
            }
        }
    }
    fields
        .into_iter()
        .map(|(kind, _, _, frames)| RasterSequence::new(kind, frames).map_err(AppError::Internal))
        .collect()
}

/// Add wind and current from a local routing store as one undoable import.
#[tauri::command(async)]
pub fn import_zarr(state: tauri::State<'_, AppState>, path: String) -> Result<ProjectSummary> {
    zarr_import(&state, path)
}

/// Implementation shared with headless callers.
pub fn zarr_import(state: &AppState, path: String) -> Result<ProjectSummary> {
    let path = PathBuf::from(path);
    with_session(state, |session| {
        let open = session.require_open()?;
        let sequences = read(&path)?;
        crate::import::add_layers(open, &path, sequences, true)?;
        Ok(ProjectSummary::of(open))
    })
}

/// Create a project whose grid, clock, and layers come from a routing store.
#[tauri::command(async)]
pub fn new_project_from_zarr(
    state: tauri::State<'_, AppState>,
    path: String,
    discard_unsaved: bool,
) -> Result<ProjectSummary> {
    zarr_project(&state, path, discard_unsaved)
}

/// Implementation shared with headless callers.
pub fn zarr_project(
    state: &AppState,
    path: String,
    discard_unsaved: bool,
) -> Result<ProjectSummary> {
    let path = PathBuf::from(path);
    with_session(state, |session| {
        crate::projects::refuse_to_discard(session, discard_unsaved)?;
        let sequences = read(&path)?;
        let settings = crate::import::settings_for(&sequences)
            .ok_or_else(|| AppError::Internal("Zarr has no vector frames".into()))?;
        let name = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "Untitled".into());
        let mut open = OpenProject::created(Project::new(name, settings));
        crate::import::add_layers(&mut open, &path, sequences, true)?;
        open.history = Default::default();
        let summary = ProjectSummary::of(&open);
        session.open = Some(open);
        Ok(summary)
    })
}

//! Local routing Zarr imports use the same raster, history, and editing path
//! as GRIB imports. Only decoding and the persisted source differ.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use rayon::prelude::*;
use ve_core::project::{FieldKind, Project};
use ve_core::raster::{MISSING, RasterFrame, RasterGrid, RasterSequence};
use ve_zarr::routing::RoutingStore;

use crate::commands::AppState;
use crate::error::{AppError, Context, Result};
use crate::projects::{ProjectSummary, with_session};
use crate::session::OpenProject;

/// The most one block of a store's samples may occupy while it is read. The
/// final immutable rasters are shared by rendering and undo, just as they are
/// for a GRIB import; this bounds only what is held on the way to them.
const BLOCK_BYTES: usize = 128 * 1024 * 1024;

/// Decode into ordinary raster sequences, retaining the native lattice.
pub fn read(path: &Path) -> Result<Vec<RasterSequence>> {
    read_reporting(path, &|_, _| {})
}

/// [`read`], with `progress(done, total)` in frames as each is finished.
pub fn read_reporting(
    path: &Path,
    progress: &(dyn Fn(usize, usize) + Sync),
) -> Result<Vec<RasterSequence>> {
    read_store(path, progress).doing("read the Zarr directory at", path.display())
}

fn read_store(
    path: &Path,
    progress: &(dyn Fn(usize, usize) + Sync),
) -> Result<Vec<RasterSequence>> {
    let internal = |e: ve_zarr::ZarrError| AppError::Internal(e.to_string());
    let store = RoutingStore::open(path).map_err(internal)?;
    let mut fields = Vec::new();
    for (kind, u, v) in [
        (FieldKind::Wind, "u10", "v10"),
        (FieldKind::Current, "ucur", "vcur"),
    ] {
        if let (Some(u), Some(v)) = (
            store.parameters.iter().position(|p| p == u),
            store.parameters.iter().position(|p| p == v),
        ) {
            fields.push((kind, u, v));
        }
    }
    let ni = store.longitude.len();
    let nj = store.latitude.len();
    let points = ni
        .checked_mul(nj)
        .filter(|points| points.checked_mul(store.parameters.len()).is_some())
        .ok_or_else(|| AppError::Internal("Zarr grid is too large".into()))?;
    let total = store.times.len() * fields.len();
    let done = AtomicUsize::new(0);
    progress(0, total);

    // The store is read along its own chunk boundaries (`RoutingStore::blocks`
    // says why), a time chunk at a time, and that chunk's frames are hashed
    // together across the pool once the last of its rows is in.
    let blocks = store.blocks(BLOCK_BYTES).map_err(internal)?;
    let pairs: Vec<(usize, usize)> = fields.iter().map(|&(_, u, v)| (u, v)).collect();
    // Routing test currents are constant. Identical frames (including empty
    // ones) share their grid instead of retaining a month's duplicate data.
    let mut grids = HashMap::<[u8; 32], Arc<RasterGrid>>::new();
    let mut sequences: Vec<Vec<RasterFrame>> = fields
        .iter()
        .map(|_| Vec::with_capacity(store.times.len()))
        .collect();
    let mut at = 0;
    while at < blocks.len() {
        let times = blocks[at].times.clone();
        let bands = blocks[at..]
            .iter()
            .take_while(|block| block.times == times)
            .count();
        let mut frames: Vec<Vec<[f32; 2]>> = (0..times.len() * fields.len())
            .map(|_| Vec::with_capacity(points))
            .collect();
        for block in &blocks[at..at + bands] {
            let row_samples = block.rows.len() * ni;
            let parameters = store.parameters.len();
            store.read_block(block).map_err(internal)?.append_pairs(
                &pairs,
                parameters,
                row_samples,
                MISSING,
                &mut frames,
            );
        }
        at += bands;

        let built: Vec<_> = frames
            .into_par_iter()
            .map(|uv| {
                let grid = RasterGrid::new(
                    ni as u32,
                    nj as u32,
                    store.longitude[0],
                    store.latitude[0],
                    (store.longitude[ni - 1] - store.longitude[0]) / (ni - 1) as f64,
                    (store.latitude[0] - store.latitude[nj - 1]) / (nj - 1) as f64,
                    uv,
                );
                progress(done.fetch_add(1, Ordering::Relaxed) + 1, total);
                grid
            })
            .collect();
        for (k, grid) in built.into_iter().enumerate() {
            let grid = grid.map_err(AppError::Internal)?;
            let t = times.start + k / fields.len();
            let grid = grids
                .entry(grid.hash)
                .or_insert_with(|| Arc::new(grid))
                .clone();
            sequences[k % fields.len()].push(RasterFrame {
                offset_hours: (store.times[t] - store.times[0]) as f64 / 3600.0,
                valid_unix_s: store.times[t],
                grid,
            });
        }
    }
    fields
        .into_iter()
        .zip(sequences)
        .map(|((kind, _, _), frames)| RasterSequence::new(kind, frames).map_err(AppError::Internal))
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
    // Read outside the session lock, as a GRIB project's file is: a month of
    // hourly wind is most of a minute, and every tile request and every edit
    // would otherwise stand behind it.
    let into = with_session(state, |session| Ok(session.require_open()?.project.id))?;
    let sequences = read(&path)?;
    with_session(state, |session| {
        let open = session.require_open()?;
        // The lock was let go of, so the project may not be the one that was
        // asked: a store must not land in whatever was opened meanwhile.
        if open.project.id != into {
            return Err(AppError::Internal(
                "the project was replaced while the Zarr directory was being read".into(),
            ));
        }
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
    // Asked before the reading as well as after it, and the reading done
    // outside the lock: see `zarr_import` and `projects::open`.
    with_session(state, |session| {
        crate::projects::refuse_to_discard(session, discard_unsaved)
    })?;
    let mut opening = state.opening.begin();
    opening.sources([path.file_name().map_or_else(
        || path.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    )]);
    let sequences = read_reporting(&path, &|done, total| opening.source(0, done, total))?;
    opening.finished();
    with_session(state, |session| {
        crate::projects::refuse_to_discard(session, discard_unsaved)?;
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

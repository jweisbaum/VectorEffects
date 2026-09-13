//! A bounded, transparent preview of selected objects, never a screen crop.

use crate::commands::AppState;
use crate::error::{AppError, Result};
use crate::projects::with_session;
use serde::Deserialize;
use ts_rs::TS;

/// A small screen-aligned grid in projected degree coordinates.
#[derive(Debug, Deserialize, TS)]
#[ts(export, export_to = "SelectionPreviewRequest.ts")]
pub struct SelectionPreviewRequest {
    /// Selected object identities.
    pub objects: Vec<u64>,
    /// The current frame.
    pub step: u32,
    /// Reject a preview from a stale document.
    pub revision: u64,
    /// First column's western edge.
    pub west: f64,
    /// First row's northern edge, in projected degrees.
    pub north: f64,
    /// Projected width and height in degrees.
    pub across: f64,
    /// Vertical extent in projected degrees.
    pub down: f64,
    /// Frozen map space: 1 equirectangular, 2 Mercator, 3 Miller.
    pub space: u8,
    /// Pixel columns, at most 512.
    pub width: u32,
    /// Pixel rows, at most 512.
    pub height: u32,
}

/// Raw little-endian float words per pixel: u, v, coverage, kind (1 wind).
#[tauri::command]
pub async fn selection_preview(
    state: tauri::State<'_, AppState>,
    request: SelectionPreviewRequest,
) -> Result<tauri::ipc::Response> {
    if request.width == 0
        || request.height == 0
        || request.width > 512
        || request.height > 512
        || request.objects.len() > 2048
        || ![request.west, request.north, request.across, request.down]
            .iter()
            .all(|v| v.is_finite())
        || request.across <= 0.0
        || request.down <= 0.0
        || !(1..=3).contains(&request.space)
    {
        return Err(AppError::BadOption {
            field: "selection preview",
            value: "invalid preview bounds".to_owned(),
        });
    }
    let project = with_session(&state, |session| {
        let open = session.require_open()?;
        if crate::projects::ProjectSummary::of(open).revision != request.revision {
            return Err(AppError::BadOption {
                field: "selection preview",
                value: "the document changed".to_owned(),
            });
        }
        Ok(open.project.clone())
    })?;
    tauri::async_runtime::spawn_blocking(move || {
        let ids: Vec<_> = request
            .objects
            .iter()
            .map(|id| crate::document::object_id(*id))
            .collect();
        let scenes = ve_render::scene::selection_scenes(&project, request.step, &ids);
        let space = ve_render::aeqd::Space::from_choice(request.space);
        let mut bytes = Vec::with_capacity((request.width * request.height * 16) as usize);
        for row in 0..request.height {
            let lat = space.lat_of(
                request.north - (f64::from(row) + 0.5) / f64::from(request.height) * request.down,
            );
            for col in 0..request.width {
                let lon = request.west
                    + (f64::from(col) + 0.5) / f64::from(request.width) * request.across;
                let point = ve_core::LonLat::new(lon, lat)?;
                let sample = ve_render::cpu::selection_sample(&scenes, point);
                let kind = if sample.kind == ve_core::FieldKind::Wind {
                    1.0_f32
                } else {
                    0.0
                };
                for value in [sample.uv.u, sample.uv.v, sample.coverage, kind] {
                    bytes.extend_from_slice(&value.to_le_bytes());
                }
            }
        }
        Ok(tauri::ipc::Response::new(bytes))
    })
    .await
    .map_err(|error| AppError::Internal(error.to_string()))?
}

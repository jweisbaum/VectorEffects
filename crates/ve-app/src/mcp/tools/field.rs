//! The field group (spec.md 8.8): sampling, capture/paste and the macro
//! library.

use rmcp::handler::server::wrapper::{Json, Parameters};
use rmcp::{tool, tool_router};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::Manager;

use super::{ToolError, VectorEffects};
use crate::commands::AppState;
use crate::projects::ProjectSummary;

/// The largest `points.len() * steps.len()` `field_sample` accepts.
///
/// Each step flattens the whole project once (`sample_points_at_step`); at
/// this bound a request is at most a few thousand samples plus however many
/// flattens `steps` asks for, not the minutes of whole-scene work an
/// unbounded grid would be (invariant 6).
const MAX_FIELD_SAMPLES: usize = 4096;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SampleParams {
    /// `[lon, lat]` pairs.
    pub points: Vec<[f64; 2]>,
    pub steps: Vec<u32>,
    /// "wind" or "current"; null samples the composite.
    pub kind: Option<String>,
}
#[derive(Debug, Serialize, JsonSchema)]
pub struct Sample {
    pub lon: f64,
    pub lat: f64,
    pub step: u32,
    pub speed_mps: f64,
    pub azimuth_toward_deg: f64,
    pub u_mps: f64,
    pub v_mps: f64,
    pub defined: bool,
}
#[derive(Debug, Serialize, JsonSchema)]
pub struct Samples {
    pub samples: Vec<Sample>,
}
#[derive(Debug, Deserialize, JsonSchema)]
pub struct CaptureParams {
    /// A region as the interface draws it: `{"kind":"rect","centre":[lon,lat],"half_width_deg":..,"half_height_deg":..}`,
    /// `{"kind":"disc","centre":[lon,lat],"radius_deg":..}`, or
    /// `{"kind":"polygon","points":[[lon,lat],...]}` (`RegionShape` in
    /// capture.rs).
    pub region: Value,
    pub step: u32,
    /// "wind" or "current"; used only as a fallback when the project has no
    /// visible field layer at all.
    pub kind: Option<String>,
}
#[derive(Debug, Deserialize, JsonSchema)]
pub struct PasteParams {
    pub lon: Option<f64>,
    pub lat: Option<f64>,
    pub step: u32,
    pub layer: Option<u64>,
    #[serde(default)]
    pub still: bool,
}
#[derive(Debug, Deserialize, JsonSchema)]
pub struct MacroInsertParams {
    pub id: String,
    pub lon: f64,
    pub lat: f64,
    pub step: u32,
    pub layer: Option<u64>,
}

#[tool_router(router = tool_router_field, vis = "pub(crate)")]
impl<R: tauri::Runtime> VectorEffects<R> {
    #[tool(
        description = "The evaluated field at points and steps: speed m/s, azimuth toward (degrees clockwise from north), u east, v north, and whether the cell is defined. Refused above 4096 points*steps."
    )]
    async fn field_sample(
        &self,
        Parameters(p): Parameters<SampleParams>,
    ) -> std::result::Result<Json<Samples>, ToolError> {
        // Refused before any allocation or session lock: each step flattens
        // the whole project (`ve_render::scene::flatten`), so an unbounded
        // points*steps grid is minutes of a starved interface from one call
        // (invariant 6).
        let total = p.points.len().saturating_mul(p.steps.len());
        if total > MAX_FIELD_SAMPLES {
            return Err(ToolError::Refused(format!(
                "field_sample: {total} points*steps requested, over the {MAX_FIELD_SAMPLES} limit; ask for fewer points or steps"
            )));
        }
        let samples = self
            .run("field_sample", move |app| {
                let state = app.state::<AppState>();
                let points: Vec<(f64, f64)> =
                    p.points.iter().map(|&[lon, lat]| (lon, lat)).collect();
                let mut out = Vec::with_capacity(total);
                // One flatten per step, not per (point, step) pair: a flatten
                // is a whole-scene walk, so this is the difference between
                // `steps.len()` flattens and `points.len() * steps.len()`.
                for &step in &p.steps {
                    let at_step = crate::commands::sample_points_at_step(
                        state.inner(),
                        &points,
                        step,
                        p.kind.clone(),
                    )?;
                    for (&[lon, lat], s) in p.points.iter().zip(at_step) {
                        let az = s.azimuth_toward_deg.to_radians();
                        out.push(Sample {
                            lon,
                            lat,
                            step,
                            speed_mps: s.speed_mps,
                            azimuth_toward_deg: s.azimuth_toward_deg,
                            u_mps: s.speed_mps * az.sin(),
                            v_mps: s.speed_mps * az.cos(),
                            defined: s.defined,
                        });
                    }
                }
                Ok(out)
            })
            .await?;
        Ok(Json(Samples { samples }))
    }

    #[tool(
        description = "Captures the field inside a region at a step onto the clipboard, for field_paste. Drops any copied objects."
    )]
    async fn field_capture(
        &self,
        Parameters(p): Parameters<CaptureParams>,
    ) -> std::result::Result<Json<crate::capture::CaptureState>, ToolError> {
        let region: crate::capture::RegionShape = serde_json::from_value(p.region)
            .map_err(|e| ToolError::Refused(format!("region: {e}")))?;
        self.run("field_capture", move |app| {
            crate::capture::capture_region(app.state(), region, p.step, p.kind)
        })
        .await
        .map(Json)
    }

    #[tool(
        description = "Pastes the captured field at lon/lat (or where it was captured), from a step, onto a layer. still true pastes one frame."
    )]
    async fn field_paste(
        &self,
        Parameters(p): Parameters<PasteParams>,
    ) -> std::result::Result<Json<ProjectSummary>, ToolError> {
        self.write("field_paste", false, move |app| {
            crate::capture::paste_capture(app.state(), p.lon, p.lat, p.step, p.layer, p.still)
        })
        .await
        .map(Json)
    }

    #[tool(description = "The macro library: id, name, kind, frames, footprint.")]
    async fn macro_list(
        &self,
    ) -> std::result::Result<Json<crate::macros::MacroLibrary>, ToolError> {
        self.run("macro_list", |app| {
            crate::macros::macro_library(app.state())
        })
        .await
        .map(Json)
    }

    #[tool(description = "Inserts a macro from the library at lon/lat starting at a step.")]
    async fn macro_insert(
        &self,
        Parameters(p): Parameters<MacroInsertParams>,
    ) -> std::result::Result<Json<ProjectSummary>, ToolError> {
        self.write("macro_insert", false, move |app| {
            crate::macros::insert_macro(app.state(), p.id, p.lon, p.lat, p.step, p.layer)
        })
        .await
        .map(Json)
    }
}

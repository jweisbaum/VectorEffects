//! The files group (spec.md 8.8): import, export and the export progress
//! relay.

use rmcp::handler::server::wrapper::{Json, Parameters};
use rmcp::{tool, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;
use tauri::Manager;

use super::{Done, ToolError, VectorEffects};
use crate::projects::ProjectSummary;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct PathParams {
    pub path: String,
}
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ImageParams {
    pub path: String,
    /// `[west, north, east, south]` to place an image with no georeference
    /// of its own; null uses the file's.
    pub view: Option<[f64; 4]>,
}
#[derive(Debug, Deserialize, JsonSchema)]
pub struct HistoryParams {
    /// Archive names as the History panel lists them: "era5",
    /// "globcurrent".
    pub archives: Vec<String>,
    pub start_unix_s: i64,
    pub end_unix_s: i64,
    #[serde(default)]
    pub set_start_time: bool,
}
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ExportParams {
    pub path: String,
    /// Reference time, UTC.
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
}

#[tool_router(router = tool_router_files, vis = "pub(crate)")]
impl<R: tauri::Runtime> VectorEffects<R> {
    #[tool(
        description = "Imports a GRIB2 file as a new layer of the open project. Slow for large files; no progress is reported, and the call returns when the import finishes."
    )]
    async fn import_grib(
        &self,
        Parameters(p): Parameters<PathParams>,
    ) -> std::result::Result<Json<ProjectSummary>, ToolError> {
        self.write("import_grib", false, move |app| {
            crate::import::import_grib(app.state(), p.path)
        })
        .await
        .map(Json)
    }

    #[tool(description = "Adds an image layer (PNG, JPEG, GeoTIFF). Display only; never exported.")]
    async fn import_image(
        &self,
        Parameters(p): Parameters<ImageParams>,
    ) -> std::result::Result<Json<ProjectSummary>, ToolError> {
        self.write("import_image", false, move |app| {
            crate::image::import_image(app.state(), p.path, p.view)
        })
        .await
        .map(Json)
    }

    #[tool(
        description = "Downloads historical wind or current (ERA5, GlobCurrent) over HTTPS into a layer. The one tool that reaches the network, and only when called."
    )]
    async fn import_history(
        &self,
        Parameters(p): Parameters<HistoryParams>,
        ctx: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> std::result::Result<Json<ProjectSummary>, ToolError> {
        let relay = self.relay_progress::<crate::history::HistoryProgress>(
            "history://progress",
            ctx,
            |h| {
                (
                    f64::from(h.done),
                    Some(f64::from(h.total)),
                    Some(h.archive.clone()),
                )
            },
        );
        let out = self
            .write("import_history", false, move |app| {
                crate::history::import_history(
                    app.clone(),
                    p.archives,
                    p.start_unix_s,
                    p.end_unix_s,
                    p.set_start_time,
                )
            })
            .await;
        relay.stop();
        out.map(Json)
    }

    #[tool(
        description = "Exports the project as GRIB2 to path. Refuses an existing file. Progress is reported; export_cancel stops it."
    )]
    async fn export_grib(
        &self,
        Parameters(p): Parameters<ExportParams>,
        ctx: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> std::result::Result<Json<crate::export::ExportResult>, ToolError> {
        let request = crate::export::ExportRequest {
            path: p.path,
            year: p.year,
            month: p.month,
            day: p.day,
            hour: p.hour,
        };
        let relay =
            self.relay_progress::<crate::export::ExportProgress>("export://progress", ctx, |e| {
                (f64::from(e.step), Some(f64::from(e.total)), None)
            });
        let out = self
            .run("export_grib", move |app| {
                crate::export::export_grib(app.clone(), app.state(), app.state(), request)
            })
            .await;
        relay.stop();
        out.map(Json)
    }

    #[tool(
        description = "Exports the project as a Zarr V3 directory at path, in the routing layout: a Float16 data array (time, param, latitude, longitude) with param u10, v10, ucur, vcur, latitude from 90 and longitude from -180, three-day by 10 degree Zstd chunks in ocean-basin shards on a rectilinear grid, NaN where uncovered. Refuses an existing directory."
    )]
    async fn export_zarr(
        &self,
        Parameters(p): Parameters<ExportParams>,
        ctx: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> std::result::Result<Json<crate::export::ExportZarrResult>, ToolError> {
        let request = crate::export::ExportZarrRequest {
            path: p.path,
            year: p.year,
            month: p.month,
            day: p.day,
            hour: p.hour,
        };
        let relay =
            self.relay_progress::<crate::export::ExportProgress>("export://progress", ctx, |e| {
                (f64::from(e.step), Some(f64::from(e.total)), None)
            });
        let out = self
            .run("export_zarr", move |app| {
                crate::export::export_zarr(app.clone(), app.state(), app.state(), request)
            })
            .await;
        relay.stop();
        out.map(Json)
    }

    #[tool(description = "Asks a running export to stop.")]
    async fn export_cancel(&self) -> std::result::Result<Json<Done>, ToolError> {
        self.run("export_cancel", |app| {
            crate::export::cancel_export(app.state());
            Ok(())
        })
        .await?;
        Ok(Json(Done { ok: true }))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use tauri::{Emitter, Listener};

    use super::super::ProgressRelay;

    /// A relay stops listening when it is dropped, not only when `stop` is
    /// called. The tool future dropped at its `.await` — a client that
    /// disconnects or cancels mid-export — never reaches `stop`, and a
    /// listener left behind would forward every later export to a peer
    /// nobody is reading, once per leak, for the life of the process.
    #[test]
    fn a_dropped_relay_stops_listening() {
        let app = tauri::test::mock_app();
        let seen = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&seen);
        let relay = ProgressRelay {
            app: app.handle().clone(),
            id: Some(app.listen("export://progress", move |_| {
                counted.fetch_add(1, Ordering::Relaxed);
            })),
        };

        let _ = app.emit("export://progress", 1_u32);
        assert_eq!(
            seen.load(Ordering::Relaxed),
            1,
            "a live relay should hear the event"
        );

        // Dropped, never stopped: the one path `stop` cannot cover.
        drop(relay);
        let _ = app.emit("export://progress", 2_u32);
        assert_eq!(
            seen.load(Ordering::Relaxed),
            1,
            "a dropped relay must not still be listening"
        );
    }
}

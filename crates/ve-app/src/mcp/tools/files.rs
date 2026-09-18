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
    /// An absolute path, or one beginning with `~/`.
    pub path: String,
}
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ImageParams {
    /// An absolute path, or one beginning with `~/`.
    pub path: String,
    /// `[west, north, east, south]` to place an image with no georeference
    /// of its own; null uses the file's.
    pub view: Option<[f64; 4]>,
}
/// What an archive holds, which is how a client asks for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum HistoryField {
    /// ERA5 10 m wind.
    Wind,
    /// GlobCurrent total surface current.
    Current,
}
#[derive(Debug, Deserialize, JsonSchema)]
pub struct HistoryParams {
    /// What to download: "wind", "current", or both. Each becomes a layer.
    pub fields: Vec<HistoryField>,
    /// The first hour wanted, ISO 8601 UTC: "2024-09-24T00:00Z". A bare
    /// date is that day's 00:00.
    pub start: String,
    /// The last hour wanted, ISO 8601 UTC. A bare date is that day's 23:00,
    /// so start "2024-09-24" and end "2024-09-27" is four whole days.
    pub end: String,
    /// Resize the timeline to the range first, so that every step from
    /// `start` to `end` is downloaded: step_count becomes
    /// (end - start) / step_hours + 1. Default true. It never shrinks a
    /// project that already holds objects.
    pub fit_timeline: Option<bool>,
    /// Stamp step 0 with the first hour downloaded, which is what labels
    /// the timeline with real dates and what export_grib takes its
    /// reference time from. Default true.
    pub set_start_time: Option<bool>,
}
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ExportParams {
    /// Where to write: an absolute path, or one beginning with `~/`. A
    /// relative path is refused, since it would land in the application's
    /// own folder and not the caller's.
    pub path: String,
    /// The reference time of step 0, UTC. Leave all four out to use the
    /// project's start time, which import_history sets.
    pub year: Option<u16>,
    pub month: Option<u8>,
    pub day: Option<u8>,
    pub hour: Option<u8>,
}

/// A path a client named, made safe to hand to a command.
///
/// The application's working directory is wherever it was launched from —
/// its own bundle, or under `tauri dev` the crate's source folder — so a
/// relative path lands somewhere the caller has never heard of while the
/// result says only the name it gave. A test agent exported
/// `hurricane.grib2`, told its user the file was "in the current
/// directory", and it was in `crates/ve-app`.
pub(crate) fn absolute(path: &str) -> std::result::Result<String, ToolError> {
    let expanded = match path.strip_prefix("~/") {
        Some(rest) => directories::BaseDirs::new()
            .map(|dirs| dirs.home_dir().join(rest))
            .ok_or_else(|| ToolError::Refused("this account has no home folder".to_owned()))?,
        None => std::path::PathBuf::from(path),
    };
    if !expanded.is_absolute() {
        return Err(ToolError::Refused(format!(
            "{path:?} is a relative path, and the application's folder is not yours: give an absolute path, or one beginning with ~/"
        )));
    }
    Ok(expanded.to_string_lossy().into_owned())
}

/// An instant a client wrote, as Unix seconds. `end_of_day` decides what a
/// bare date means: the day's first hour, or its last.
pub(crate) fn parse_utc(
    field: &str,
    text: &str,
    end_of_day: bool,
) -> std::result::Result<i64, ToolError> {
    use chrono::{DateTime, NaiveDate, NaiveDateTime};
    let text = text.trim();
    if let Ok(instant) = DateTime::parse_from_rfc3339(text) {
        return Ok(instant.timestamp());
    }
    let bare = text.trim_end_matches(['Z', 'z']).trim_end();
    for format in [
        "%Y-%m-%dT%H:%M:%S",
        "%Y-%m-%dT%H:%M",
        "%Y-%m-%d %H:%M:%S",
        "%Y-%m-%d %H:%M",
    ] {
        if let Ok(instant) = NaiveDateTime::parse_from_str(bare, format) {
            return Ok(instant.and_utc().timestamp());
        }
    }
    if let Ok(date) = NaiveDate::parse_from_str(bare, "%Y-%m-%d") {
        let hour = if end_of_day { 23 } else { 0 };
        if let Some(instant) = date.and_hms_opt(hour, 0, 0) {
            return Ok(instant.and_utc().timestamp());
        }
    }
    Err(ToolError::Refused(format!(
        "{field}: {text:?} is not a time this reads; write it as 2024-09-24T00:00Z"
    )))
}

/// The steps a timeline needs for every `step_hours` from `start` to `end`.
pub(crate) fn steps_covering(start_unix_s: i64, end_unix_s: i64, step_hours: u32) -> u64 {
    let stride = i64::from(step_hours.max(1)) * 3600;
    let start = start_unix_s.div_euclid(3600) * 3600;
    ((end_unix_s - start).max(0) / stride) as u64 + 1
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
        let path = absolute(&p.path)?;
        self.write("import_grib", false, move |app| {
            crate::import::import_grib(app.state(), path)
        })
        .await
        .map(Json)
    }

    #[tool(description = "Adds an image layer (PNG, JPEG, GeoTIFF). Display only; never exported.")]
    async fn import_image(
        &self,
        Parameters(p): Parameters<ImageParams>,
    ) -> std::result::Result<Json<ProjectSummary>, ToolError> {
        let path = absolute(&p.path)?;
        self.write("import_image", false, move |app| {
            crate::image::import_image(app.state(), path, p.view)
        })
        .await
        .map(Json)
    }

    #[tool(
        description = "Today's date, and the archives of REAL PAST WEATHER that import_history downloads from — observed wind (ERA5 reanalysis) and observed ocean current (GlobCurrent) — with the first and last hour each holds right now. Call it before import_history: both trail the present by days, and a range past an archive's last hour is refused. Reaches the network."
    )]
    async fn history_archives(
        &self,
    ) -> std::result::Result<Json<crate::history::HistoryArchives>, ToolError> {
        self.run("history_archives", |_| crate::history::history_archives())
            .await
            .map(Json)
    }

    #[tool(
        description = "Downloads REAL PAST WEATHER into the open project: the observed global wind (ERA5) and/or ocean current (GlobCurrent) for a range of dates, as one layer each. This is the tool for any request about weather that actually happened — a named hurricane or storm, a race, a voyage, 'the weather on' a date: find the dates, then call this; never draw such weather by hand. Needs an open project (project_new; its step_hours is the spacing of what is downloaded). By default it resizes the timeline to the range and stamps step 0 with the start, so afterwards export_grib needs only a path. At most 240 steps per call: ten days hourly, a month at 3 h, two months at 6 h. Takes from seconds to a few minutes and reports progress. The one tool, with history_archives, that reaches the network."
    )]
    async fn import_history(
        &self,
        Parameters(p): Parameters<HistoryParams>,
        ctx: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> std::result::Result<Json<ProjectSummary>, ToolError> {
        let start = parse_utc("start", &p.start, false)?;
        let end = parse_utc("end", &p.end, true)?;
        if end < start {
            return Err(ToolError::Refused(format!(
                "end {:?} is before start {:?}",
                p.end, p.start
            )));
        }
        if p.fields.is_empty() {
            return Err(ToolError::Refused(
                "fields is empty: ask for \"wind\", \"current\" or both".to_owned(),
            ));
        }
        let archives: Vec<String> = ve_zarr::Archive::ALL
            .into_iter()
            .filter(|archive| {
                p.fields.iter().any(|field| match field {
                    HistoryField::Wind => crate::history::field_of(*archive) == "wind",
                    HistoryField::Current => crate::history::field_of(*archive) == "current",
                })
            })
            .map(|archive| archive.id().to_owned())
            .collect();
        let fit = p.fit_timeline.unwrap_or(true);
        let set_start_time = p.set_start_time.unwrap_or(true);

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
                let open = crate::projects::current_project(app.state())?.ok_or(
                    crate::error::AppError::NoProjectOpen,
                )?;
                let needed = steps_covering(start, end, open.step_hours);
                let most = u64::from(ve_core::project::MAX_STEPS);
                if needed > most {
                    let hours = (end - start) / 3600;
                    return Err(crate::error::AppError::BadOption {
                        field: "range",
                        value: format!(
                            "{hours} hours at this project's {} h step is {needed} steps, and a project holds {most}. Make the project with a longer step_hours (it cannot be changed afterwards), or import a shorter range",
                            open.step_hours
                        ),
                    });
                }
                let needed = needed as u32;
                // Grown freely; shrunk only when nothing could be lost to
                // it, since shrinking drops the keys past the new end.
                if fit
                    && needed != open.step_count
                    && (needed > open.step_count || open.object_count == 0)
                {
                    crate::animation::set_step_count(app.state(), needed)?;
                }
                crate::history::import_history(
                    app.clone(),
                    archives,
                    start,
                    end,
                    set_start_time,
                )
            })
            .await;
        relay.stop();
        out.map(Json)
    }

    #[tool(
        description = "Exports the project as a GRIB2 file: every step, u and v, wind and/or current. `path` must be absolute (or begin with ~/) and must not exist. The reference time defaults to the project's start time, which import_history sets; give year/month/day/hour for a drawn project that has none. Progress is reported; export_cancel stops it."
    )]
    async fn export_grib(
        &self,
        Parameters(p): Parameters<ExportParams>,
        ctx: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> std::result::Result<Json<crate::export::ExportResult>, ToolError> {
        let (path, [year, month, day, hour]) = self.export_target(&p).await?;
        let request = crate::export::ExportRequest {
            path,
            year: year as u16,
            month: month as u8,
            day: day as u8,
            hour: hour as u8,
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
        let (path, [year, month, day, hour]) = self.export_target(&p).await?;
        let request = crate::export::ExportZarrRequest {
            path,
            year: year as u16,
            month: month as u8,
            day: day as u8,
            hour: hour as u8,
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

impl<R: tauri::Runtime> VectorEffects<R> {
    /// Where an export goes and the reference time it carries: the four
    /// numbers the caller gave, or the project's own start time.
    async fn export_target(
        &self,
        p: &ExportParams,
    ) -> std::result::Result<(String, [u32; 4]), ToolError> {
        use chrono::{Datelike, Timelike};
        let path = absolute(&p.path)?;
        if let (Some(year), Some(month), Some(day)) = (p.year, p.month, p.day) {
            let hour = p.hour.unwrap_or(0);
            return Ok((
                path,
                [
                    u32::from(year),
                    u32::from(month),
                    u32::from(day),
                    u32::from(hour),
                ],
            ));
        }
        if p.year.is_some() || p.month.is_some() || p.day.is_some() || p.hour.is_some() {
            return Err(ToolError::Refused(
                "give year, month and day together (hour defaults to 0), or none of them to use the project's start time".to_owned(),
            ));
        }
        let start = self
            .run("project_status", |app| {
                crate::projects::current_project(app.state())
            })
            .await?
            .ok_or(ToolError::Refused(
                crate::error::AppError::NoProjectOpen.to_string(),
            ))?
            .start_unix_s
            .and_then(|unix_s| chrono::DateTime::from_timestamp(unix_s, 0))
            .ok_or_else(|| {
                ToolError::Refused(
                    "this project has no start time to take a reference time from: give year, month, day and hour (UTC), or set one with timeline_set".to_owned(),
                )
            })?;
        Ok((
            path,
            [
                start.year().max(0) as u32,
                start.month(),
                start.day(),
                start.hour(),
            ],
        ))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use tauri::{Emitter, Listener};

    use super::super::ProgressRelay;
    use super::{absolute, parse_utc, steps_covering};

    fn unix(text: &str, end_of_day: bool) -> i64 {
        match parse_utc("start", text, end_of_day) {
            Ok(unix_s) => unix_s,
            Err(_) => panic!("{text:?} should parse"),
        }
    }

    /// 2024-09-24T00:00Z is 1 727 136 000: 19 990 days after the epoch
    /// (54 years of 365 days, 14 leap days from 1972 to 2024, and the 267
    /// days of 2024 before the 24th of September), times 86 400.
    #[test]
    fn a_time_is_read_however_a_client_is_likely_to_write_it() {
        const MIDNIGHT: i64 = 19_990 * 86_400;
        assert_eq!(MIDNIGHT, 1_727_136_000);
        for text in [
            "2024-09-24T00:00Z",
            "2024-09-24T00:00:00Z",
            "2024-09-24T00:00:00+00:00",
            "2024-09-24T02:00:00+02:00",
            "2024-09-24 00:00",
            "2024-09-24T00:00",
            " 2024-09-24 ",
        ] {
            assert_eq!(unix(text, false), MIDNIGHT, "{text:?}");
        }
        assert_eq!(unix("2024-09-24T06:00Z", false), MIDNIGHT + 6 * 3600);
        // A bare end date is the whole of that day.
        assert_eq!(unix("2024-09-24", true), MIDNIGHT + 23 * 3600);
        for text in ["", "last tuesday", "2024-13-01", "24/09/2024"] {
            assert!(parse_utc("start", text, false).is_err(), "{text:?}");
        }
    }

    /// Four whole days six-hourly are sixteen steps, the first at 00:00 on
    /// the first day and the last at 18:00 on the fourth; hourly they are
    /// ninety-six.
    #[test]
    fn a_timeline_covers_a_range_with_a_step_at_each_end() {
        let (start, end) = (unix("2024-09-24", false), unix("2024-09-27", true));
        assert_eq!(steps_covering(start, end, 6), 16);
        assert_eq!(steps_covering(start, end, 1), 96);
        assert_eq!(steps_covering(start, end, 24), 4);
        assert_eq!(
            steps_covering(start, start, 6),
            1,
            "one instant is one step"
        );
        // Minutes into the hour start at the hour, as the import does.
        assert_eq!(steps_covering(start + 1800, start + 6 * 3600, 6), 2);
    }

    #[test]
    fn a_path_is_absolute_or_under_home_and_never_relative() {
        let root = if cfg!(windows) {
            "C:\\out\\a.grib2"
        } else {
            "/out/a.grib2"
        };
        assert_eq!(absolute(root).ok().as_deref(), Some(root));
        let home = absolute("~/Desktop/a.grib2").ok().unwrap_or_default();
        assert!(std::path::Path::new(&home).is_absolute(), "{home}");
        assert!(home.ends_with("a.grib2") && !home.contains('~'), "{home}");
        for relative in ["a.grib2", "./a.grib2", "out/a.grib2", "~a.grib2"] {
            assert!(absolute(relative).is_err(), "{relative}");
        }
    }

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

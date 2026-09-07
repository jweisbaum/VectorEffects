//! Importing a range of hours from the history archives (spec.md 4.10, M38).
//!
//! The user names a start and an end; the hours between are fetched from the
//! public archives — ERA5 for 10 m wind, GlobCurrent for the total surface
//! current — written to a GRIB2 file each, and imported as layers. From then
//! on they *are* GRIB layers in every way that matters: the same frames, the
//! same speed filter, the same eraser, the same edit objects, the same
//! macros, the same export, and the same re-read from the file when the
//! project is reopened. What they keep beyond a forecast layer is where they
//! came from.
//!
//! **This is the one path that reaches the network**, and only because the
//! user pressed the button (invariant 5, as amended for spec.md 4.10). It
//! fetches then and never again: the file on disk is what the project reads
//! afterwards, so a project opened without a connection shows its history
//! layers like any other.
//!
//! # Only the hours the project can show
//!
//! A step shows an imported message only where the file has one for that
//! step's own forecast hour (spec.md 4.8, D48), so an hour that falls between
//! two steps is never drawn and never exported. On a three-hourly project two
//! hours in every three are exactly that, and fetching them would be minutes
//! of a stranger's bandwidth spent on data nothing can display. So the import
//! strides by the project's step, and reads the hours the project has steps
//! for and no others.
//!
//! # Why it goes through a file
//!
//! A layer's field is read from a file (spec.md 4.8, invariants 1 and 2), and
//! writing one is what makes a history layer behave exactly as a forecast
//! layer does rather than nearly. It also means the hours are fetched once:
//! the archives are slow and their chunks are large, and re-reading them on
//! every open would make a project unopenable away from a connection.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::Serialize;
use ts_rs::TS;
use ve_core::Command;
use ve_core::document::Layer;
use ve_grib::writer::{GridSpec, MessageSpec, Parameter, ReferenceTime, message_masked};
use ve_zarr::{Archive, Field, Utc, Variable};

use crate::commands::AppState;
use crate::error::{AppError, Result};
use crate::projects::{ProjectSummary, with_session};

/// The grid every archive hands its fields back on: ERA5's 0.25 degree global
/// grid, north to south from the pole and east from the prime meridian, which
/// is GRIB2 scanning mode 0 exactly.
const GRID: GridSpec = GridSpec {
    ni: ve_zarr::NI as u32,
    nj: ve_zarr::NJ as u32,
    micro_degrees: 250_000,
};

/// Bits per packed value. Sixteen is a millimetre per second over any wind or
/// current the archives hold, far finer than either is measured to.
const BITS: u8 = 16;

/// The most steps one import will fetch from one archive.
///
/// The wait is proportional to what is *fetched*, not to how long a span the
/// user named, and striding by the project's step is what separates the two:
/// 240 hourly steps is ten days, and the same 240 steps at six-hourly is two
/// months. A request for more is refused, with its count, before anything is
/// transferred rather than abandoned halfway — the user can ask for the next
/// stretch as a second import, and the two land as separate layers they can
/// see and delete.
///
/// The project's own step count bounds this already
/// ([`ve_core::project::MAX_STEPS`] is the same number), so this is a guard
/// rather than the binding limit. It is stated here because it is this
/// module's promise about how long an import can run.
pub const MAX_FETCHED_STEPS: usize = 240;

/// Seconds in an hour. The archives are hourly and so is everything here.
const HOUR: i64 = 3600;

/// How far along a fetch is, emitted as the `history://progress` event.
///
/// A history import is minutes of silence otherwise, and an indeterminate
/// spinner cannot tell a slow fetch from a stalled one. The steps are known
/// before the first byte moves, so the bar is a real fraction rather than an
/// animation.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "HistoryProgress.ts")]
pub struct HistoryProgress {
    /// Which archive is being read, as [`Archive::label`] names it.
    pub archive: String,
    /// Steps fetched so far, across every archive of this import.
    pub done: u32,
    /// Steps this import will fetch in total, across every archive.
    pub total: u32,
}

/// What the frontend asks for.
#[derive(Debug, Clone)]
pub struct HistoryRequest {
    /// Archive identifiers, as [`Archive::id`] spells them.
    pub archives: Vec<String>,
    /// First hour wanted, in Unix seconds.
    pub start_unix_s: i64,
    /// Last hour wanted, in Unix seconds.
    pub end_unix_s: i64,
}

/// Imports the hours of a range that the project has steps for.
#[tauri::command(async)]
pub fn import_history(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    archives: Vec<String>,
    start_unix_s: i64,
    end_unix_s: i64,
) -> Result<ProjectSummary> {
    use tauri::Emitter;
    history_import(
        &state,
        &HistoryRequest {
            archives,
            start_unix_s,
            end_unix_s,
        },
        |progress| {
            let _ = app.emit("history://progress", progress);
        },
    )
}

/// Implementation of [`import_history`], callable without a Tauri handle.
pub fn history_import(
    state: &AppState,
    request: &HistoryRequest,
    mut on_progress: impl FnMut(HistoryProgress),
) -> Result<ProjectSummary> {
    let archives = archives_of(request)?;

    // The times the project can actually show. A step serves an imported
    // message only where the file has one for that step's own forecast hour
    // (spec.md 4.8, D48), so on a three-hourly project two hours in every
    // three could never be drawn — and fetching them would be minutes spent
    // on data the application throws away.
    let (step_hours, step_count) = with_session(state, |session| {
        let settings = &session.require_open()?.project.settings;
        Ok((settings.step_hours.hours(), settings.step_count))
    })?;
    let wanted = wanted_hours(request, step_hours, step_count)?;
    tracing::info!(
        archives = ?request.archives,
        steps = wanted.len(),
        step_hours,
        "history import starting"
    );

    let directory = state.paths.history_dir.clone();
    std::fs::create_dir_all(&directory)?;

    // Fetched and written first, outside the session lock: this takes
    // minutes, and the document has to stay readable while it runs.
    let total = (wanted.len() * archives.len()) as u32;
    let mut done = 0u32;
    let mut written = Vec::new();
    for archive in archives {
        // Reported before the archive's first chunk as well as after each
        // one: opening a store costs seconds, and a bar that only moves on
        // the first completed step is another silence at the start.
        on_progress(HistoryProgress {
            archive: archive.label().to_owned(),
            done,
            total,
        });
        let path = fetch_to_file(archive, request, &wanted, &directory, |fetched| {
            on_progress(HistoryProgress {
                archive: archive.label().to_owned(),
                done: done + fetched,
                total,
            });
        })?;
        done += wanted.len() as u32;
        written.push((archive, path));
    }

    with_session(state, |session| {
        let open = session.require_open()?;
        let mut layers = Vec::with_capacity(written.len());
        for (archive, path) in &written {
            layers.push(history_layer(*archive, path, request)?);
        }

        // The first hour any of them holds. A project with no start time
        // takes it, exactly as a project made from a GRIB takes its file's
        // (spec.md 4.8) — and through the same command the timeline's own
        // control uses, so one undo takes the whole import back, start time
        // included, rather than leaving the timeline stamped with hours that
        // are no longer there.
        let earliest = layers
            .iter()
            .filter_map(|layer| layer.raster.as_ref()?.frames.first())
            .map(|frame| frame.valid_unix_s)
            .min();
        let mut commands = Vec::with_capacity(layers.len() + 1);
        if let (None, Some(start)) = (open.project.settings.start_unix_s, earliest) {
            commands.push(Command::SetStartTime {
                before: None,
                after: Some(start),
            });
        }
        // Each layer lands on top of the last: the index is where it will be
        // once the layers before it in the batch have been added.
        let base = open.project.layers.len();
        for (at, layer) in layers.into_iter().enumerate() {
            commands.push(Command::AddLayer {
                index: base + at,
                layer: Box::new(layer),
            });
        }
        let command = if commands.len() == 1 {
            commands.remove(0)
        } else {
            Command::Batch {
                label: "Import history".to_owned(),
                commands,
            }
        };
        let (project, history) = (&mut open.project, &mut open.history);
        history.push(project, command)?;
        open.touch();
        Ok(ProjectSummary::of(session.require_open()?))
    })
}

/// The archives a request names, in the order they are read.
fn archives_of(request: &HistoryRequest) -> Result<Vec<Archive>> {
    if request.archives.is_empty() {
        return Err(AppError::BadOption {
            field: "archives",
            value: "no archive was asked for".to_owned(),
        });
    }
    request
        .archives
        .iter()
        .map(|id| {
            Archive::parse(id).ok_or_else(|| AppError::BadOption {
                field: "archives",
                value: format!("{id} is not an archive this reads"),
            })
        })
        .collect()
}

/// The hours of a request that land on one of the project's steps.
///
/// The project's first step is the file's first hour (spec.md 4.8), so the
/// hours it can show are the range's start and every `step_hours` after it,
/// and there are at most `step_count` of them however long the range is. Both
/// bounds matter: the stride is what stops a three-hourly project fetching
/// three times what it can draw, and the count is what stops a range running
/// past the end of the timeline fetching hours with no step to land on.
fn wanted_hours(request: &HistoryRequest, step_hours: u32, step_count: u32) -> Result<Vec<i64>> {
    if request.end_unix_s < request.start_unix_s {
        return Err(bad_range("the start is after the end"));
    }
    // A step of zero hours is not a project the app can make; guarding it
    // here keeps this a total function rather than a division by zero.
    let stride = i64::from(step_hours.max(1)) * HOUR;
    let start = request.start_unix_s.div_euclid(HOUR) * HOUR;
    let hours: Vec<i64> = (0..i64::from(step_count))
        .map(|step| start + step * stride)
        .take_while(|hour| *hour <= request.end_unix_s)
        .collect();
    if hours.is_empty() {
        return Err(bad_range("that range holds none of the project's steps"));
    }
    if hours.len() > MAX_FETCHED_STEPS {
        return Err(bad_range(&format!(
            "{} steps is more than one import fetches; ask for {MAX_FETCHED_STEPS} or fewer",
            hours.len()
        )));
    }
    Ok(hours)
}

/// Fetches the wanted hours from one archive and writes them as GRIB2.
///
/// `wanted` is the project's own step times, so nothing between two steps is
/// read: the archive is walked, and an hour it holds that no step lands on is
/// skipped without being fetched. An hour a step wants and the archive lacks
/// is skipped too, leaving that step with no message, which is what a step
/// with no message means everywhere else (spec.md 4.8, D48).
///
/// The file is named for the archive and the range, so asking for the same
/// hours twice rewrites one file rather than filling the directory, and so
/// someone looking in the directory can tell what each file holds.
fn fetch_to_file(
    archive: Archive,
    request: &HistoryRequest,
    wanted: &[i64],
    directory: &Path,
    mut on_step: impl FnMut(u32),
) -> Result<PathBuf> {
    let source = archive.open().map_err(zarr_failed)?;
    // Every hour the archive holds in the range, by hour. Asked for as a
    // range rather than hour by hour because that is the call that knows the
    // store's coverage, and its refusal names what the store actually has —
    // which is the message worth showing when a range is out of reach.
    let held: std::collections::BTreeMap<i64, ve_zarr::Step> = source
        .steps_in_range(at_hour(request.start_unix_s), at_hour(request.end_unix_s))
        .map_err(zarr_failed)?
        .into_iter()
        .map(|step| (step.valid_time.hours_since_unix_epoch(), step))
        .collect();

    // Forecast hours count from the range's first wanted hour rather than
    // from whatever the archive happens to hold first, so two archives
    // fetched together agree step for step. The importer rebases a file onto
    // its own first message (spec.md 4.8), so this matters only when an
    // archive is missing the range's opening hours — and then it is the two
    // layers agreeing with each other that is worth more than either one
    // starting where it was asked to.
    let anchor = wanted[0].div_euclid(HOUR);
    let reference = reference_time(anchor * HOUR)?;

    let mut bytes = Vec::new();
    let mut written = 0u32;
    for (at, unix_s) in wanted.iter().enumerate() {
        let hour = unix_s.div_euclid(HOUR);
        // A step the archive has no hour for is simply left out, and that
        // step then shows nothing — which is what a step with no message
        // means everywhere else (spec.md 4.8, D48).
        let Some(step) = held.get(&hour) else {
            continue;
        };
        let forecast_hour = u32::try_from(hour - anchor)
            .map_err(|_| bad_range("the archive returned an hour before the range's start"))?;
        bytes.extend(encode_hour(
            GRID,
            reference,
            forecast_hour,
            &source.read_step(step).map_err(zarr_failed)?,
        )?);
        written += 1;
        on_step(at as u32 + 1);
    }
    if written == 0 {
        return Err(bad_range(&format!(
            "{} holds no step of that range",
            archive.label()
        )));
    }

    let path = directory.join(format!(
        "{}-{}-{}.grib2",
        archive.id(),
        request.start_unix_s,
        request.end_unix_s
    ));
    std::fs::write(&path, &bytes)?;
    tracing::info!(
        archive = archive.id(),
        steps = written,
        bytes = bytes.len(),
        path = %path.display(),
        "history steps written"
    );
    Ok(path)
}

/// Writes one hour's fields as GRIB2 messages.
///
/// Two messages per field, u then v, each with the points the archive has no
/// value for marked absent rather than packed as a number. GlobCurrent has no
/// current over land, and a land value of zero is not "no current" but "dead
/// calm" — the distinction D58 exists to keep.
fn encode_hour(
    grid: GridSpec,
    reference: ReferenceTime,
    forecast_hour: u32,
    fields: &[Field],
) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    for field in fields {
        for (parameter, values) in components(field) {
            let spec = MessageSpec {
                parameter,
                grid,
                reference_time: reference,
                forecast_hour,
                // 255 is "missing". No originating-centre code says "read out
                // of a public Zarr archive", and claiming one that does not
                // apply would be worse than saying nothing.
                centre: 255,
                bits: BITS,
            };
            bytes.extend(message_masked(&spec, values)?);
        }
    }
    Ok(bytes)
}

/// The two messages a field is written as.
fn components(field: &Field) -> [(Parameter, &[f32]); 2] {
    match field.variable {
        Variable::Wind10m => [
            (Parameter::WindU, field.u.as_slice()),
            (Parameter::WindV, field.v.as_slice()),
        ],
        Variable::SurfaceCurrent => [
            (Parameter::CurrentU, field.u.as_slice()),
            (Parameter::CurrentV, field.v.as_slice()),
        ],
    }
}

/// Reads a written file back as a layer.
///
/// Through the ordinary reader, so a history layer holds exactly what a
/// forecast layer holds and there is no second decoding path to keep in step.
/// The grid is a regular 0.25 degree lat/lon lattice, which is what the app
/// samples natively, so nothing is resampled.
fn history_layer(archive: Archive, path: &Path, request: &HistoryRequest) -> Result<Layer> {
    let imported = ve_grib::import::read_file(path, None)?;
    for skipped in &imported.skipped {
        tracing::warn!(
            path = %path.display(),
            message = skipped.index + 1,
            reason = %skipped.reason,
            "history message skipped on import"
        );
    }
    let sequence = imported
        .sequences
        .into_iter()
        .next()
        .ok_or_else(|| AppError::Internal(format!("{} produced no field", archive.label())))?;
    Ok(Layer::from_history(
        archive.label(),
        path.to_path_buf(),
        Arc::new(sequence),
        archive.id(),
        request.start_unix_s,
        request.end_unix_s,
    ))
}

/// The hour a Unix time falls in.
fn at_hour(unix_s: i64) -> Utc {
    Utc::from_hours_since_unix_epoch(unix_s.div_euclid(HOUR))
}

/// The GRIB reference time a Unix instant names, on the hour.
fn reference_time(unix_s: i64) -> Result<ReferenceTime> {
    let utc = at_hour(unix_s);
    Ok(ReferenceTime {
        year: u16::try_from(utc.year)
            .map_err(|_| bad_range("the project starts outside the years GRIB2 can state"))?,
        month: utc.month,
        day: utc.day,
        hour: utc.hour,
        minute: 0,
        second: 0,
    })
}

fn bad_range(why: &str) -> AppError {
    AppError::BadOption {
        field: "range",
        value: why.to_owned(),
    }
}

fn zarr_failed(error: ve_zarr::ZarrError) -> AppError {
    AppError::Internal(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(start: i64, end: i64) -> HistoryRequest {
        HistoryRequest {
            archives: vec!["era5-wind".to_owned()],
            start_unix_s: start,
            end_unix_s: end,
        }
    }

    /// The point of M38's follow-up: a project that shows every third hour
    /// downloads every third hour. Fetching the two hours between them is
    /// minutes of transfer for data no step could ever draw (spec.md 4.8,
    /// D48).
    #[test]
    fn only_the_hours_the_project_steps_on_are_fetched() {
        // A day of range on a three-hourly project.
        assert_eq!(
            wanted_hours(&request(0, HOUR * 24), 3, 240).expect("a plan"),
            (0..=8).map(|k| k * 3 * HOUR).collect::<Vec<_>>(),
            "every third hour, both ends included"
        );
        // Hourly is every hour, which is what it was before.
        assert_eq!(
            wanted_hours(&request(0, HOUR * 24), 1, 240)
                .expect("a plan")
                .len(),
            25
        );
        // Six-hourly is a quarter of the three-hourly count.
        assert_eq!(
            wanted_hours(&request(0, HOUR * 24), 6, 240).expect("a plan"),
            vec![0, 6 * HOUR, 12 * HOUR, 18 * HOUR, 24 * HOUR]
        );
    }

    /// A range longer than the timeline is cut at the timeline: an hour past
    /// the last step has no step to land on, so fetching it is waste of the
    /// same kind.
    #[test]
    fn no_more_hours_than_the_project_has_steps() {
        let hours = wanted_hours(&request(0, HOUR * 1000), 3, 8).expect("a plan");
        assert_eq!(hours.len(), 8, "eight steps, not a thousand hours");
        assert_eq!(hours.last(), Some(&(7 * 3 * HOUR)));
    }

    /// Both ends are inclusive, so a start and an end in the same hour is one
    /// hour of data and not none.
    #[test]
    fn the_range_is_inclusive_at_both_ends() {
        assert_eq!(
            wanted_hours(&request(0, 0), 1, 240).expect("a plan"),
            vec![0]
        );
        assert_eq!(
            wanted_hours(&request(0, HOUR), 1, 240).expect("a plan"),
            vec![0, HOUR]
        );
        // An end one second short of the next step does not reach it.
        assert_eq!(
            wanted_hours(&request(0, HOUR * 3 - 1), 3, 240).expect("a plan"),
            vec![0]
        );
    }

    /// A start part-way through an hour names the hour it falls in: the
    /// archives are hourly, and a step at 00:30 would match nothing.
    #[test]
    fn a_start_is_taken_to_the_hour_it_falls_in() {
        assert_eq!(
            wanted_hours(&request(1800, HOUR * 6), 3, 240).expect("a plan"),
            vec![0, 3 * HOUR, 6 * HOUR]
        );
    }

    /// The cap counts what is fetched, not how long a span was named. Only a
    /// project with more steps than the app allows can reach it, so this is a
    /// guard — but the message still has to say what it refused.
    #[test]
    fn more_steps_than_one_import_fetches_is_refused_with_the_count() {
        let over = (MAX_FETCHED_STEPS + 5) as u32;
        let error = wanted_hours(&request(0, HOUR * 100_000), 1, over).expect_err("refused");
        let message = format!("{error}");
        assert!(message.contains(&over.to_string()), "{message}");
        assert!(
            message.contains(&MAX_FETCHED_STEPS.to_string()),
            "{message}"
        );
    }

    /// The same 240 steps is ten days hourly and two months six-hourly. The
    /// cap is on the fetching, so a coarser project may reach further back.
    #[test]
    fn a_coarser_project_may_ask_for_a_longer_span() {
        let month = HOUR * 24 * 30;
        assert!(
            wanted_hours(&request(0, 2 * month), 6, 240).is_ok(),
            "two months six-hourly is 240 steps"
        );
    }

    #[test]
    fn a_backwards_range_is_refused() {
        assert!(wanted_hours(&request(HOUR * 5, 0), 1, 240).is_err());
    }

    /// An archive the reader does not know is named in the refusal rather
    /// than silently dropped, which would import half of what was asked for
    /// and report success.
    #[test]
    fn an_unknown_archive_is_refused_by_name() {
        let mut asked = request(0, HOUR);
        asked.archives.push("gfs".to_owned());
        let error = archives_of(&asked).expect_err("refused");
        assert!(format!("{error}").contains("gfs"), "{error}");
    }

    #[test]
    fn no_archive_at_all_is_refused() {
        let mut asked = request(0, HOUR);
        asked.archives.clear();
        assert!(archives_of(&asked).is_err());
    }

    /// The reference time is what every forecast hour in the file counts
    /// from, so it has to be the file's first hour exactly. An hour out puts
    /// every hour of the import on the wrong step, and nothing later in the
    /// pipeline could notice.
    #[test]
    fn the_reference_time_is_the_hour_it_is_given() {
        // 2026-09-07T13:00Z. 20 703 days from the epoch to that date, by the
        // civil-date arithmetic ve-zarr proves against known dates.
        let unix_s = (20_703 * 24 + 13) * HOUR;
        let reference = reference_time(unix_s).expect("a time");
        assert_eq!(
            (
                reference.year,
                reference.month,
                reference.day,
                reference.hour
            ),
            (2026, 9, 7, 13)
        );
        assert_eq!((reference.minute, reference.second), (0, 0));
        reference.validate().expect("a valid reference time");
    }

    /// The grid the messages are written on is the grid the archives read on.
    /// A mismatch would be a file of the right size holding the wrong
    /// geography.
    #[test]
    fn the_grid_matches_the_archives() {
        assert_eq!(u64::from(GRID.ni), ve_zarr::NI);
        assert_eq!(u64::from(GRID.nj), ve_zarr::NJ);
        assert_eq!(GRID.point_count() as usize, ve_zarr::POINTS_PER_STEP);
        // 0.25 degrees: the spacing 1440 columns of whole globe implies.
        assert_eq!(
            u64::from(GRID.ni) * u64::from(GRID.micro_degrees),
            360_000_000
        );
    }

    /// The whole pipeline, minus the fetch: hours encoded as this writes
    /// them, read back by the importer the layer will use, and landing on
    /// the steps their forecast hours name.
    ///
    /// This is the join everything else depends on. The archive hands back
    /// fields; the layer reads a file; and nothing between them is checked
    /// by either half's own tests. A three-by-two grid stands in for the
    /// global one so the round trip costs nothing.
    #[test]
    fn encoded_hours_read_back_on_the_steps_they_name() {
        let grid = GridSpec {
            ni: 3,
            nj: 2,
            micro_degrees: 90_000_000,
        };
        // 2020-01-01T00:00Z, the range's first hour, with the second three
        // hours after it: an off-by-one in either would show.
        let reference = reference_time(parse("2020-01-01T00:00")).expect("a time");

        // A wind field with a hole in it: the fourth node has no value, the
        // way a current field has none over land.
        let mut u = vec![3.0_f32, 4.0, 5.0, 6.0, 7.0, 8.0];
        let v = vec![0.0_f32; 6];
        u[3] = f32::NAN;
        let field = Field {
            variable: Variable::Wind10m,
            u,
            v,
        };

        let mut bytes = Vec::new();
        for hour in [0_u32, 3] {
            bytes.extend(
                encode_hour(grid, reference, hour, std::slice::from_ref(&field)).expect("messages"),
            );
        }

        let dir = std::env::temp_dir().join(format!("ve-history-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a directory");
        let path = dir.join("hours.grib2");
        std::fs::write(&path, &bytes).expect("written");

        let imported = ve_grib::import::read_file(&path, None).expect("read back");
        assert_eq!(imported.sequences.len(), 1, "one field kind");
        let sequence = &imported.sequences[0];
        assert_eq!(sequence.kind, ve_core::FieldKind::Wind);

        // Both hours land where their forecast hours say, and nothing sits
        // between them: a step the file has no message for shows nothing
        // (spec.md 4.8, D48).
        let offsets: Vec<f64> = sequence.frames.iter().map(|f| f.offset_hours).collect();
        assert_eq!(offsets, vec![0.0, 3.0]);
        assert!(sequence.frame_at(1.0).is_none(), "no message at hour 1");

        // The values survive, and the hole comes back as a hole rather than
        // as a calm (D58).
        let frame = sequence.frame_at(0.0).expect("the first hour");
        // Sampled at the nodes themselves: 90 degree spacing from the pole
        // and the prime meridian, which is where the writer puts them.
        let at = |lon: f64, lat: f64| frame.grid.sample(lon, lat);
        assert!(
            (at(0.0, 90.0).expect("a value").u - 3.0).abs() < 0.01,
            "{:?}",
            at(0.0, 90.0)
        );
        assert!(
            (at(180.0, 90.0).expect("a value").u - 5.0).abs() < 0.01,
            "{:?}",
            at(180.0, 90.0)
        );
        assert!(
            at(0.0, 0.0).is_none(),
            "the masked node reads as no value, not as a calm (D58)"
        );

        std::fs::remove_file(&path).ok();
    }

    /// A UTC hour as Unix seconds, for the tests above.
    fn parse(text: &str) -> i64 {
        Utc::parse(text).expect("a time").hours_since_unix_epoch() * HOUR
    }

    /// Wind is written as wind and current as current. Swapping them would
    /// produce a file that decodes cleanly and shows a six-knot gale.
    #[test]
    fn each_variable_is_written_as_its_own_parameter() {
        let field = |variable| Field {
            variable,
            u: vec![1.0; 4],
            v: vec![2.0; 4],
        };
        let wind = field(Variable::Wind10m);
        assert_eq!(
            components(&wind).map(|(p, _)| p),
            [Parameter::WindU, Parameter::WindV]
        );
        let current = field(Variable::SurfaceCurrent);
        assert_eq!(
            components(&current).map(|(p, _)| p),
            [Parameter::CurrentU, Parameter::CurrentV]
        );
        // u first, v second, in that order, for both.
        assert_eq!(components(&wind)[0].1, [1.0; 4]);
        assert_eq!(components(&wind)[1].1, [2.0; 4]);
    }
}

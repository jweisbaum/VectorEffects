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
//! # Why it goes through a file
//!
//! A layer's field is read from a file (spec.md 4.8, invariants 1 and 2), and
//! writing one is what makes a history layer behave exactly as a forecast
//! layer does rather than nearly. It also means the hours are fetched once:
//! the archives are slow and their chunks are large, and re-reading them on
//! every open would make a project unopenable away from a connection.

use std::path::{Path, PathBuf};
use std::sync::Arc;

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

/// The most hours one import will fetch.
///
/// Ten days of hourly wind and current is a few hundred megabytes over the
/// wire and a long wait on a domestic connection. A longer range is refused,
/// with its length, before anything is fetched rather than abandoned halfway:
/// the user can ask for the next ten days as a second import, and the two
/// land as separate layers they can see and delete.
pub const MAX_HOURS: i64 = 240;

/// Seconds in an hour. The archives are hourly and so is everything here.
const HOUR: i64 = 3600;

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

/// Imports every hour of a range from each archive asked for.
#[tauri::command(async)]
pub fn import_history(
    state: tauri::State<'_, AppState>,
    archives: Vec<String>,
    start_unix_s: i64,
    end_unix_s: i64,
) -> Result<ProjectSummary> {
    history_import(
        &state,
        &HistoryRequest {
            archives,
            start_unix_s,
            end_unix_s,
        },
    )
}

/// Implementation of [`import_history`], callable without a Tauri handle.
pub fn history_import(state: &AppState, request: &HistoryRequest) -> Result<ProjectSummary> {
    let wanted = archives_of(request)?;
    let hours = hours_asked_for(request)?;
    tracing::info!(archives = ?request.archives, hours, "history import starting");

    let directory = state.paths.history_dir.clone();
    std::fs::create_dir_all(&directory)?;

    // Fetched and written first, outside the session lock: this takes
    // minutes, and the document has to stay readable while it runs.
    let mut written = Vec::new();
    for archive in wanted {
        written.push((archive, fetch_to_file(archive, request, &directory)?));
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

/// How many hours a request covers, refusing a range that is backwards or
/// longer than one import fetches.
fn hours_asked_for(request: &HistoryRequest) -> Result<i64> {
    if request.end_unix_s < request.start_unix_s {
        return Err(bad_range("the start is after the end"));
    }
    let hours = (request.end_unix_s - request.start_unix_s) / HOUR + 1;
    if hours > MAX_HOURS {
        return Err(bad_range(&format!(
            "{hours} hours is more than one import fetches; ask for {MAX_HOURS} or fewer"
        )));
    }
    Ok(hours)
}

/// Fetches every hour of the range from one archive and writes it as GRIB2.
///
/// The file is named for the archive and the range, so asking for the same
/// hours twice rewrites one file rather than filling the directory, and so
/// someone looking in the directory can tell what each file holds.
fn fetch_to_file(archive: Archive, request: &HistoryRequest, directory: &Path) -> Result<PathBuf> {
    let source = archive.open().map_err(zarr_failed)?;
    let steps = source
        .steps_in_range(at_hour(request.start_unix_s), at_hour(request.end_unix_s))
        .map_err(zarr_failed)?;
    let Some(first) = steps.first() else {
        return Err(bad_range(&format!(
            "{} holds no hour of that range",
            archive.label()
        )));
    };

    // Forecast hours count from the file's own first hour, which is what
    // makes this a forecast file like any other: the importer aligns a file's
    // first message with the project's first step (spec.md 4.8), and a
    // history layer is to behave exactly as an imported one does.
    let anchor = first.valid_time.hours_since_unix_epoch();
    let reference = reference_time(anchor * HOUR)?;
    let mut bytes = Vec::new();
    for step in &steps {
        let forecast_hour = u32::try_from(step.valid_time.hours_since_unix_epoch() - anchor)
            .map_err(|_| bad_range("the archive returned an hour before the range's start"))?;
        bytes.extend(encode_hour(
            GRID,
            reference,
            forecast_hour,
            &source.read_step(step).map_err(zarr_failed)?,
        )?);
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
        hours = steps.len(),
        bytes = bytes.len(),
        path = %path.display(),
        "history hours written"
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

    /// The range is bounded, and the refusal says by how much, before
    /// anything is fetched: an import that would run for an hour should be
    /// declined at the dialog rather than abandoned halfway (M38).
    #[test]
    fn a_range_longer_than_the_cap_is_refused_with_its_length() {
        let hours = MAX_HOURS + 10;
        let error = hours_asked_for(&request(0, HOUR * (hours - 1))).expect_err("refused");
        let message = format!("{error}");
        assert!(message.contains(&hours.to_string()), "{message}");
        assert!(message.contains(&MAX_HOURS.to_string()), "{message}");
    }

    /// Both ends are inclusive, so a start and an end in the same hour is one
    /// hour of data and not zero.
    #[test]
    fn the_range_is_inclusive_at_both_ends() {
        assert_eq!(hours_asked_for(&request(0, 0)).expect("a range"), 1);
        assert_eq!(hours_asked_for(&request(0, HOUR)).expect("a range"), 2);
        assert_eq!(
            hours_asked_for(&request(0, HOUR * (MAX_HOURS - 1))).expect("a range"),
            MAX_HOURS
        );
    }

    #[test]
    fn a_backwards_range_is_refused() {
        assert!(hours_asked_for(&request(HOUR * 5, 0)).is_err());
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

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "test code; clippy's allow-in-tests does not reach tests/"
)]
//! The near-real-time import (spec.md 4.10): the last N days up to now, one
//! held layer per product, and the timeline set to the period.
//!
//! Nothing here reaches the network: the products are opened through a
//! closure, and these tests hand it sources that live in memory.

use ve_app::commands::AppState;
use ve_app::document;
use ve_app::edit;
use ve_app::nrt::{self, NrtRequest, Period};
use ve_app::paths::AppPaths;
use ve_app::projects::{self, NewProjectRequest};
use ve_zarr::{Field, FieldSource, Product, Step, Utc, Variable, ZarrError};

const HOUR: i64 = 3600;
const DAY: i64 = 86_400;
/// 2026-10-01T00Z. From 1970 to 2026 is fifty-six years with fourteen leap
/// days, 20 454 days, and 1 October is day 273 of 2026: 20 727 days.
const OCT_1: i64 = 20_727 * DAY;
/// 2026-10-02T15:40Z, the moment the user presses the button.
const NOW: i64 = OCT_1 + DAY + 15 * HOUR + 40 * 60;

struct TempRoot(std::path::PathBuf);

impl TempRoot {
    fn new(label: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "ve-nrt-{}-{label}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or_default()
        ));
        std::fs::create_dir_all(&dir).expect("temp root");
        Self(dir)
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn app(root: &TempRoot, step_hours: u32, step_count: u32) -> AppState {
    let state = AppState::new(AppPaths::in_directory(&root.0).expect("paths"));
    projects::create(
        &state,
        NewProjectRequest {
            name: "NRT".to_owned(),
            field_kind: "wind".to_owned(),
            resolution: "1.0".to_owned(),
            step_hours,
            step_count,
        },
        false,
    )
    .expect("create");
    state
}

/// A product held in memory: a field at each of `times`, eastward, as fast
/// in m/s as its hour of the day plus one.
struct Fake {
    variable: Variable,
    /// Hours since the Unix epoch, sorted.
    times: Vec<i64>,
    /// Hours listed but not written yet: they read as fields with no value.
    unwritten: Vec<i64>,
    /// Whether each node differs from the next, so a message has something
    /// to pack: a constant field packs to no data section at all.
    textured: bool,
}

impl Fake {
    fn boxed(variable: Variable, unix_times: &[i64]) -> Box<dyn FieldSource> {
        Box::new(Self {
            variable,
            times: unix_times.iter().map(|t| t / HOUR).collect(),
            unwritten: Vec::new(),
            textured: false,
        })
    }

    fn boxed_textured(variable: Variable, unix_times: &[i64]) -> Box<dyn FieldSource> {
        Box::new(Self {
            variable,
            times: unix_times.iter().map(|t| t / HOUR).collect(),
            unwritten: Vec::new(),
            textured: true,
        })
    }

    fn boxed_with_unwritten(
        variable: Variable,
        unix_times: &[i64],
        unwritten: &[i64],
    ) -> Box<dyn FieldSource> {
        Box::new(Self {
            variable,
            times: unix_times.iter().map(|t| t / HOUR).collect(),
            unwritten: unwritten.iter().map(|t| t / HOUR).collect(),
            textured: false,
        })
    }
}

impl FieldSource for Fake {
    fn name(&self) -> &'static str {
        "fake"
    }
    fn variables(&self) -> Vec<Variable> {
        vec![self.variable]
    }
    fn coverage(&self) -> Option<(Utc, Utc)> {
        Some((
            Utc::from_hours_since_unix_epoch(*self.times.first()?),
            Utc::from_hours_since_unix_epoch(*self.times.last()?),
        ))
    }
    fn step_at(&self, time: Utc) -> Option<Step> {
        ve_zarr::source::step_at_hour(&self.times, time)
    }
    fn steps_in_range(&self, start: Utc, end: Utc) -> ve_zarr::Result<Vec<Step>> {
        ve_zarr::source::steps_between(&self.times, start, end, "fake")
    }
    fn read_step(&self, step: &Step) -> ve_zarr::Result<Vec<Field>> {
        if self
            .unwritten
            .contains(&step.valid_time.hours_since_unix_epoch())
        {
            return Ok(vec![Field {
                variable: self.variable,
                u: vec![f32::NAN; ve_zarr::POINTS_PER_STEP],
                v: vec![f32::NAN; ve_zarr::POINTS_PER_STEP],
            }]);
        }
        let speed = (step.valid_time.hour + 1) as f32;
        let u = if self.textured {
            (0..ve_zarr::POINTS_PER_STEP)
                .map(|k| speed + (k % 97) as f32 * 0.01)
                .collect()
        } else {
            vec![speed; ve_zarr::POINTS_PER_STEP]
        };
        Ok(vec![Field {
            variable: self.variable,
            u,
            v: vec![0.0; ve_zarr::POINTS_PER_STEP],
        }])
    }
}

fn hourly(from: i64, to: i64) -> Vec<i64> {
    (0..=(to - from) / HOUR).map(|h| from + h * HOUR).collect()
}

fn request(products: &[&str], days: u32, set_start_time: bool, extend: bool) -> NrtRequest {
    NrtRequest {
        products: products.iter().map(|p| (*p).to_owned()).collect(),
        days,
        set_start_time,
        extend_timeline: extend,
    }
}

/// What the timeline's frame row shows for the layer at `at`.
fn covered(state: &AppState, at: usize) -> Vec<bool> {
    let tree = document::tree(state, 0).expect("tree");
    tree.layers[at]
        .grib
        .as_ref()
        .expect("a field layer")
        .covered_steps
        .clone()
}

fn steps_true(range: std::ops::Range<usize>, of: usize) -> Vec<bool> {
    (0..of).map(|s| range.contains(&s)).collect()
}

/// The period, worked by hand: three days before 2 October is 29 September
/// at midnight, the end is the hour the button was pressed in, and an hourly
/// timeline needs 72 + 15 + 1 steps for it.
#[test]
fn the_period_runs_from_midnight_n_days_back_to_the_current_hour() {
    assert_eq!(
        nrt::period(NOW, 3, 1).expect("three days"),
        Period {
            start_unix_s: OCT_1 - 2 * DAY,
            end_unix_s: OCT_1 + DAY + 15 * HOUR,
            steps: 88,
        }
    );
    // A second either side of midnight is a different day to count back from.
    let before = nrt::period(OCT_1 - 1, 1, 1).expect("before midnight");
    let after = nrt::period(OCT_1 + 1, 1, 1).expect("after midnight");
    assert_eq!(before.start_unix_s, OCT_1 - 2 * DAY);
    assert_eq!(after.start_unix_s, OCT_1 - DAY);
    assert_eq!(before.end_unix_s, OCT_1 - HOUR, "23:00, the hour it was");
    assert_eq!(after.end_unix_s, OCT_1);
    assert_eq!((before.steps, after.steps), (48, 25));
    // No step is past the current hour, whatever the step size.
    for step_hours in [1, 3, 6, 24] {
        let period = nrt::period(NOW, 2, step_hours).expect("two days");
        let last = period.start_unix_s + i64::from(period.steps - 1) * i64::from(step_hours) * HOUR;
        assert!(last <= period.end_unix_s, "{step_hours} h: {period:?}");
        assert!(last + i64::from(step_hours) * HOUR > period.end_unix_s);
    }
}

/// 240 steps is the most a project holds: ten days of hours at 15:40 is 256.
#[test]
fn a_period_longer_than_the_timeline_can_hold_is_refused() {
    let refused = nrt::period(NOW, 10, 1).expect_err("256 steps").to_string();
    assert!(refused.contains("240"), "{refused}");
    assert_eq!(nrt::period(NOW, 9, 1).expect("nine days").steps, 232);
    // 28 days at three hours: (672 + 15) / 3 + 1.
    assert_eq!(nrt::period(NOW, 28, 3).expect("28 days").steps, 230);
    assert!(nrt::period(NOW, 0, 1).is_err(), "no days is no period");
}

/// Only the times that land on a step are fetched, and each once: a daily
/// product is its midnights whatever the timeline's step, and an hourly one
/// on a three-hourly timeline is every third hour.
#[test]
fn a_product_is_fetched_at_the_times_the_timeline_can_show() {
    let period = nrt::period(NOW, 3, 1).expect("period");
    let midnights: Vec<i64> = (0..4).map(|d| period.start_unix_s + d * DAY).collect();
    assert_eq!(nrt::wanted_times(&period, 1, 88, 24), midnights);
    assert_eq!(nrt::wanted_times(&period, 6, 15, 24), midnights);
    assert_eq!(nrt::wanted_times(&period, 24, 4, 24), midnights);

    let third: Vec<i64> = (0..30)
        .map(|k| period.start_unix_s + k * 3 * HOUR)
        .collect();
    assert_eq!(nrt::wanted_times(&period, 3, 30, 1), third);
    assert_eq!(nrt::wanted_times(&period, 1, 88, 1).len(), 88);

    // A timeline shorter than the period cuts the list where it ends.
    assert_eq!(nrt::wanted_times(&period, 3, 10, 1), third[..10]);
    assert_eq!(nrt::wanted_times(&period, 1, 24, 24), midnights[..1]);
    assert_eq!(nrt::wanted_times(&period, 1, 25, 24), midnights[..2]);
}

/// The whole import: two products, the timeline set to the period and
/// lengthened to hold it, and one undo taking all of it back.
#[test]
fn an_import_adds_held_layers_and_sets_the_timeline() {
    let root = TempRoot::new("import");
    let state = app(&root, 1, 24);
    let start = OCT_1; // one day back from 2 October
    let open = |product: Product| -> ve_zarr::Result<Box<dyn FieldSource>> {
        Ok(match product {
            // Daily, and a day behind: it has 30 September and 1 October.
            Product::Duacs => Fake::boxed(Variable::SurfaceCurrent, &[start - DAY, start]),
            // Hourly, to 20:00 on 1 October.
            _ => Fake::boxed(
                Variable::SurfaceCurrent,
                &hourly(start - DAY, start + 20 * HOUR),
            ),
        })
    };
    let mut seen = Vec::new();
    let outcome = nrt::nrt_import(
        &state,
        &request(&["duacs", "multiobs"], 1, true, true),
        NOW,
        open,
        |progress| seen.push((progress.done, progress.total)),
    )
    .expect("import");

    assert!(outcome.skipped.is_empty(), "{:?}", outcome.skipped);
    assert_eq!(outcome.project.start_unix_s, Some(start));
    assert_eq!(outcome.project.step_count, 40, "24 + 15 + 1 steps");
    assert_eq!(outcome.project.layer_count, 3);

    // The daily field is on every hour of its day and on none after; the
    // hourly one is on the hours it has.
    assert_eq!(covered(&state, 1), steps_true(0..24, 40));
    assert_eq!(covered(&state, 2), steps_true(0..21, 40));
    {
        let mut session = state.session.lock().expect("lock");
        let project = &session.require_open().expect("open").project;
        assert_eq!(project.layers[1].source.period_hours(), Some(24));
        assert_eq!(project.layers[2].source.period_hours(), Some(1));
        assert_eq!(project.layers[1].name, Product::Duacs.label());
    }
    // Two midnights wanted of the daily product, forty hours of the hourly.
    assert_eq!(seen.last().map(|(_, total)| *total), Some(42));

    // One undo is the whole import: layers, start and length.
    let back = edit::undo_for_test(&state).expect("undo");
    assert_eq!(back.layer_count, 1);
    assert_eq!(back.start_unix_s, None);
    assert_eq!(back.step_count, 24);
}

/// Neither box ticked: the timeline is left exactly as it was, and the fetch
/// stops where it ends.
#[test]
fn an_import_leaves_the_timeline_alone_when_not_asked() {
    let root = TempRoot::new("untouched");
    let state = app(&root, 1, 24);
    let open = |_: Product| -> ve_zarr::Result<Box<dyn FieldSource>> {
        Ok(Fake::boxed(
            Variable::Wind10m,
            &hourly(OCT_1 - DAY, OCT_1 + DAY + 10 * HOUR),
        ))
    };
    let mut total = 0;
    let outcome = nrt::nrt_import(
        &state,
        &request(&["wind-l4"], 1, false, false),
        NOW,
        open,
        |progress| total = progress.total,
    )
    .expect("import");
    assert_eq!(outcome.project.start_unix_s, None);
    assert_eq!(outcome.project.step_count, 24);
    assert_eq!(total, 24, "no hour is fetched that no step can show");
    assert_eq!(covered(&state, 1), steps_true(0..24, 24));
}

/// A fetch is minutes long and the document stays editable through it. A
/// timeline lengthened meanwhile keeps its new length: the import never
/// shortens one.
#[test]
fn an_import_never_shortens_a_timeline_that_grew_while_it_ran() {
    let root = TempRoot::new("grew");
    let state = app(&root, 1, 24);
    let grown = std::cell::Cell::new(false);
    let open = |_: Product| -> ve_zarr::Result<Box<dyn FieldSource>> {
        // The user sets the duration to sixty steps while the first product
        // is being reached.
        if !grown.replace(true) {
            ve_app::animation::resize_steps(&state, 60).expect("lengthen meanwhile");
        }
        Ok(Fake::boxed(
            Variable::SurfaceCurrent,
            &hourly(OCT_1, OCT_1 + 5 * HOUR),
        ))
    };
    let outcome = nrt::nrt_import(
        &state,
        &request(&["multiobs"], 1, true, true),
        NOW,
        open,
        |_| {},
    )
    .expect("import");
    assert_eq!(
        outcome.project.step_count, 60,
        "the period needs 40; the timeline has 60"
    );
    assert_eq!(outcome.project.layer_count, 2);
    // And the one undo takes back the layer and the start, not the sixty.
    let back = edit::undo_for_test(&state).expect("undo");
    assert_eq!(back.step_count, 60);
    assert_eq!(back.layer_count, 1);
}

/// A product missing the period's first time still lands where it belongs:
/// its second day is a day in, not on step 0.
#[test]
fn a_product_missing_its_first_time_keeps_its_place() {
    let root = TempRoot::new("late");
    let state = app(&root, 1, 24);
    let open = |_: Product| -> ve_zarr::Result<Box<dyn FieldSource>> {
        // Only 2 October: 1 October was never published.
        Ok(Fake::boxed(Variable::SurfaceCurrent, &[OCT_1 + DAY]))
    };
    nrt::nrt_import(
        &state,
        &request(&["duacs"], 1, true, true),
        NOW,
        open,
        |_| {},
    )
    .expect("import");
    let mut session = state.session.lock().expect("lock");
    let project = &session.require_open().expect("open").project;
    let frames = &project.layers[1].raster.as_ref().expect("a raster").frames;
    let offsets: Vec<f64> = frames.iter().map(|f| f.offset_hours).collect();
    assert_eq!(offsets, [0.0, 24.0]);
    assert_eq!(frames[1].valid_unix_s, OCT_1 + DAY);
    // The first day is there to hold the place and holds nothing else.
    let settings = &project.settings;
    let shown = |step: u32| {
        project.layers[1]
            .imported_frame(settings, step)
            .and_then(|frame| frame.grid.sample(30.0, 40.0))
            .map(|uv| uv.u)
    };
    assert_eq!(shown(0), None, "the missing day shows nothing");
    assert_eq!(shown(23), None);
    // Midnight's field: an eastward wind of hour 0 plus one.
    assert_eq!(shown(24), Some(1.0));
    assert_eq!(shown(39), Some(1.0));
}

/// A time the store lists and has not written yet — the new day, just after
/// midnight — is an empty read, and is written as nothing: the timeline must
/// not say the layer covers a day it shows nothing for.
#[test]
fn an_unwritten_time_is_not_a_frame() {
    let root = TempRoot::new("unwritten");
    let state = app(&root, 1, 24);
    let open = |_: Product| -> ve_zarr::Result<Box<dyn FieldSource>> {
        Ok(Fake::boxed_with_unwritten(
            Variable::SurfaceCurrent,
            &[OCT_1, OCT_1 + DAY],
            &[OCT_1 + DAY],
        ))
    };
    nrt::nrt_import(
        &state,
        &request(&["duacs"], 1, true, true),
        NOW,
        open,
        |_| {},
    )
    .expect("import");
    // Two midnights were listed; the second read empty. One frame, holding
    // its day and no longer.
    let frames = {
        let mut session = state.session.lock().expect("lock");
        let project = &session.require_open().expect("open").project;
        project.layers[1]
            .raster
            .as_ref()
            .expect("a raster")
            .frames
            .len()
    };
    assert_eq!(frames, 1);
    assert_eq!(covered(&state, 1), steps_true(0..24, 40));
}

/// A product that cannot be fetched is named and the others still arrive;
/// nothing arriving at all is an error, and adds nothing.
#[test]
fn a_product_that_fails_does_not_stop_the_others() {
    let root = TempRoot::new("partial");
    let state = app(&root, 1, 24);
    let open = |product: Product| -> ve_zarr::Result<Box<dyn FieldSource>> {
        match product {
            Product::Duacs => Err(ZarrError::Open("the catalogue is not answering".to_owned())),
            // Its newest data is a week old: nothing in the period.
            Product::WindL4 => Ok(Fake::boxed(Variable::Wind10m, &[OCT_1 - 7 * DAY])),
            _ => Ok(Fake::boxed(
                Variable::SurfaceCurrent,
                &hourly(OCT_1, OCT_1 + 5 * HOUR),
            )),
        }
    };
    let outcome = nrt::nrt_import(
        &state,
        &request(&["duacs", "wind-l4", "multiobs"], 1, true, true),
        NOW,
        open,
        |_| {},
    )
    .expect("one product arrived");
    assert_eq!(outcome.project.layer_count, 2);
    let skipped: Vec<&str> = outcome.skipped.iter().map(|s| s.product.as_str()).collect();
    assert_eq!(skipped, [Product::Duacs.label(), Product::WindL4.label()]);
    assert!(
        outcome.skipped[0].why.contains("not answering"),
        "{:?}",
        outcome.skipped
    );

    let before = document::tree(&state, 0).expect("tree").layers.len();
    let nothing = nrt::nrt_import(
        &state,
        &request(&["duacs", "wind-l4"], 1, true, true),
        NOW,
        open,
        |_| {},
    )
    .expect_err("nothing arrived")
    .to_string();
    assert!(nothing.contains("not answering"), "{nothing}");
    assert_eq!(
        document::tree(&state, 0).expect("tree").layers.len(),
        before
    );

    // And a request that names nothing, or something this does not read.
    assert!(nrt::nrt_import(&state, &request(&[], 1, true, true), NOW, open, |_| {}).is_err());
    assert!(
        nrt::nrt_import(
            &state,
            &request(&["era5-wind"], 1, true, true),
            NOW,
            open,
            |_| {}
        )
        .is_err()
    );
}

/// An SST product makes a display-only layer of temperatures: one day per
/// step-day, sampled by the readout in Celsius, a day token per step for the
/// tiles — and none of it a field. Saved and reopened, the days come back
/// from their file (spec.md 4.10, M93).
#[test]
fn an_sst_product_is_a_layer_of_temperatures_that_survives_a_reopen() {
    let root = TempRoot::new("sst");
    let state = app(&root, 1, 24);
    let open = |_: Product| -> ve_zarr::Result<Box<dyn FieldSource>> {
        // The fake's value is its hour plus one: 1 C at each midnight.
        Ok(Fake::boxed(
            Variable::SeaSurfaceTemperature,
            &[OCT_1, OCT_1 + DAY],
        ))
    };
    nrt::nrt_import(
        &state,
        &request(&["oisst"], 1, true, true),
        NOW,
        open,
        |_| {},
    )
    .expect("import");

    let tree = document::tree(&state, 0).expect("tree");
    let layer = &tree.layers[1];
    assert_eq!(layer.source, "sst");
    assert!(layer.grib.is_none(), "not a field layer");
    let sst = layer.sst.as_ref().expect("an SST view");
    assert_eq!(sst.product, "oisst");
    assert_eq!(sst.tokens.len(), 40);
    assert!(
        sst.tokens[..24]
            .iter()
            .all(|t| *t == sst.tokens[0] && t.is_some())
    );
    assert!(sst.tokens[24].is_some());
    {
        let mut session = state.session.lock().expect("lock");
        let project = &session.require_open().expect("open").project;
        assert!(project.layers[1].source.is_display_only());
        assert!(
            project.layers[1].raster.is_none(),
            "nothing for a scene to find"
        );
    }

    let at = |step| ve_app::sst::temperature_at(&state, step, -30.0, 40.0).expect("sample");
    let first = at(0).expect("a temperature");
    assert!((first - 1.0).abs() < 0.01, "{first}");
    assert_eq!(at(39).map(|t| (t * 100.0).round()), Some(100.0));

    let path = root.0.join("sst.veproj");
    projects::save_as(&state, path.to_string_lossy().into_owned()).expect("save");
    projects::close_open(&state, true).expect("close");
    projects::open(&state, path.to_string_lossy().into_owned(), true).expect("reopen");
    let again = at(0).expect("the days came back from their file");
    assert!((again - first).abs() < 1e-6);
}

/// After an SST import the top of the stack is a layer that is only drawn:
/// an object placed with no layer chosen goes to the layer beneath it, and
/// one aimed at it is refused — it can hold no field and no edit of one.
/// A product missing the period's first day shows no day there.
#[test]
fn an_sst_layer_takes_no_object_and_its_empty_first_day_is_no_day() {
    use ve_app::create::{self, Gesture, NewObject, Tool};
    let root = TempRoot::new("sst-objects");
    let state = app(&root, 1, 24);
    let open = |_: Product| -> ve_zarr::Result<Box<dyn FieldSource>> {
        // 1 October was never published: the import pads it, empty.
        Ok(Fake::boxed(Variable::SeaSurfaceTemperature, &[OCT_1 + DAY]))
    };
    nrt::nrt_import(
        &state,
        &request(&["ostia"], 1, true, true),
        NOW,
        open,
        |_| {},
    )
    .expect("import");
    let tree = document::tree(&state, 0).expect("tree");
    let sst_layer = tree.layers[1].id;
    let tokens = &tree.layers[1].sst.as_ref().expect("an SST view").tokens;
    assert!(
        tokens[..24].iter().all(Option::is_none),
        "the padded day is no day"
    );
    assert!(tokens[24].is_some());

    let brush = |layer: Option<u64>| NewObject {
        tool: Tool::Brush,
        gesture: Gesture::Stroke {
            points: vec![[-20.0, 0.0], [20.0, 0.0]],
        },
        options: Vec::new(),
        layer,
    };
    let refused = create::create(&state, brush(Some(sst_layer)))
        .expect_err("an SST layer holds no field")
        .to_string();
    assert!(refused.contains("only drawn"), "{refused}");
    create::create(&state, brush(None)).expect("the layer beneath takes it");
    let tree = document::tree(&state, 0).expect("tree");
    assert_eq!(tree.layers[0].objects.len(), 1, "on the painted layer");
    assert!(tree.layers[1].objects.is_empty());
}

/// Makes the open project regional: 160 E to 160 W, 10 S to 10 N.
fn make_regional(state: &AppState) -> ve_core::region::Region {
    let mut session = state.session.lock().expect("lock");
    let open = session.open.as_mut().expect("open");
    let region = ve_core::region::Region::snapped(
        160.0,
        -160.0,
        -10.0,
        10.0,
        false,
        open.project.settings.resolution,
    )
    .expect("a region");
    open.project.settings.region = Some(region);
    region
}

/// The files a fetch left in the history folder.
fn history_files(root: &TempRoot) -> Vec<std::path::PathBuf> {
    let mut files: Vec<_> = std::fs::read_dir(root.0.join("history"))
        .expect("the history folder")
        .map(|entry| entry.expect("an entry").path())
        .collect();
    files.sort();
    files
}

/// A regional project's fetch writes its region and nothing else (spec.md
/// 4.10, M102). The history import and this one share the writer
/// (`history::fetch_source_to_file`); this drives it through the fake
/// product because the archives are only reached over the network.
///
/// 160 E to 160 W and 10 S to 10 N at the sources' 0.25 degrees is 161 by
/// 81 nodes against the globe's 1440 by 721: 1.3 % of the points, so the
/// file is under 2 % of the global run's. Its name carries the region.
#[test]
fn a_regional_history_import_writes_a_regional_file() {
    let open = |_: Product| -> ve_zarr::Result<Box<dyn FieldSource>> {
        Ok(Fake::boxed_textured(
            Variable::SurfaceCurrent,
            &hourly(OCT_1, OCT_1 + 6 * HOUR),
        ))
    };
    let global_root = TempRoot::new("global-file");
    let global = app(&global_root, 1, 24);
    nrt::nrt_import(
        &global,
        &request(&["multiobs"], 1, true, true),
        NOW,
        open,
        |_| {},
    )
    .expect("global import");
    let regional_root = TempRoot::new("regional-file");
    let regional = app(&regional_root, 1, 24);
    let region = make_regional(&regional);
    nrt::nrt_import(
        &regional,
        &request(&["multiobs"], 1, true, true),
        NOW,
        open,
        |_| {},
    )
    .expect("regional import");

    let [global_file] = history_files(&global_root).try_into().expect("one file");
    let [regional_file] = history_files(&regional_root).try_into().expect("one file");
    let name = |p: &std::path::Path| p.file_name().unwrap().to_string_lossy().into_owned();
    assert!(!name(&global_file).contains(&region.key()));
    assert_eq!(
        name(&regional_file),
        name(&global_file).replace(".grib2", &format!("-{}.grib2", region.key())),
    );

    // Section 3 states the region: the decoder reads the file as written.
    let read = ve_grib::import::read_file(&regional_file, None).expect("reads");
    let grid = &read.sequences[0].frames[0].grid;
    assert_eq!((grid.ni, grid.nj), (161, 81));
    assert_eq!((grid.lon0, grid.lat0), (160.0, 10.0));
    let read = ve_grib::import::read_file(&global_file, None).expect("reads");
    let grid = &read.sequences[0].frames[0].grid;
    assert_eq!((grid.ni, grid.nj), (1440, 721));

    let size = |p: &std::path::Path| std::fs::metadata(p).expect("a file").len();
    let (g, r) = (size(&global_file), size(&regional_file));
    assert!(r * 50 < g, "regional {r} bytes against global {g}");
}

/// A regional project's fetched layer holds the region from the moment it
/// is fetched, as it does after a reopen (spec.md 4.8, M101). The file now
/// holds only the region snapped outward to 0.25 degrees (M102), so the
/// layer is that and not a node of the project's own 1 degree beyond it.
#[test]
fn a_regional_projects_fetched_layer_holds_its_region() {
    let root = TempRoot::new("regional");
    let state = app(&root, 1, 24);
    make_regional(&state);
    let open = |_: Product| -> ve_zarr::Result<Box<dyn FieldSource>> {
        Ok(Fake::boxed(
            Variable::SurfaceCurrent,
            &hourly(OCT_1, OCT_1 + 20 * HOUR),
        ))
    };
    nrt::nrt_import(
        &state,
        &request(&["multiobs"], 1, true, true),
        NOW,
        open,
        |_| {},
    )
    .expect("import");
    let session = state.session.lock().expect("lock");
    let project = &session.open.as_ref().expect("open").project;
    let grid = &project.layers[1].raster.as_ref().expect("a field").frames[0].grid;
    // 0.25° source, 160 E to 160 W and 10 S to 10 N: 40° by 20° at 0.25°.
    assert_eq!((grid.ni, grid.nj), (161, 81));
    assert!(grid.sample(-179.5, 0.0).is_some());
    assert!(grid.sample(0.0, 0.0).is_none());
}

//! NOAA's Blended Seawinds, near-real-time (spec.md 4.10, M96).
//!
//! NOAA CoastWatch publishes one NetCDF-4 file per day — four six-hourly
//! fields of 10 m wind at 0.25° — in a directory whose index is the only
//! catalogue there is. So the store reads that index once for the days it
//! holds, and each read fetches the day's file whole and reads it with
//! [`ve_hdf5`]; there is no subsetting service in front of it.
//!
//! **A day's file is fetched once**, however many of its four steps are
//! asked for and on however many threads: the import reads steps several
//! at a time, and an 18 MB file fetched four times over is most of an
//! import's time. The day is dropped once each of its steps has been read.
//!
//! **Only the named host is reached**, as for [`crate::erddap`]: the same
//! client, which does not follow a redirect off it.

use std::sync::{Arc, Mutex, OnceLock};

use crate::arco::cell_grid_of;
use crate::erddap::{Fetch, regrid_within};
use crate::error::{Result, ZarrError};
use crate::regrid::CellGrid;
use crate::source::{Field, FieldSource, Step, Variable, step_at_hour, steps_between};
use crate::store::{hours_from_axis, parse_time_units};
use crate::time::Utc;

/// The directory the day files are in.
pub const DIRECTORY: &str =
    "https://coastwatch.noaa.gov/data/pub0015/coastwatch/blended/wind/nrt/uvcomp/6hr/";

const PREFIX: &str = "NBSv02_wind_6hourly_";
const SUFFIX: &str = "_nrt.nc";
const NAME: &str = "Blended Seawinds";
/// The hours of a day the product has a field for.
const SLOTS: [i64; 4] = [0, 6, 12, 18];
/// Days held at once at most, should a day's steps not all be read.
const HELD_DAYS: usize = 3;

/// The days a directory index lists, as each day's midnight in hours since
/// the Unix epoch and its `YYYYMMDD`, sorted and without repeats — an index
/// names each file in its link and again in its text.
pub fn days_listed(index: &str) -> Vec<(i64, String)> {
    let mut days: Vec<(i64, String)> = index
        .match_indices(PREFIX)
        .filter_map(|(at, _)| {
            let rest = &index[at + PREFIX.len()..];
            let date = rest.get(..8)?;
            // The name ends at the suffix: `….nc.md5` beside it is not the
            // day's file, and listing a day for it fails the import.
            let after = rest[8..].strip_prefix(SUFFIX)?.bytes().next();
            let ends = after.is_none_or(|b| !(b.is_ascii_alphanumeric() || b"._-".contains(&b)));
            if !ends || !date.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            let iso = format!("{}-{}-{}T00:00", &date[..4], &date[4..6], &date[6..]);
            Some((Utc::parse(&iso)?.hours_since_unix_epoch(), date.to_owned()))
        })
        .collect();
    days.sort();
    days.dedup();
    days
}

/// One day's file, read: its own times and both components at each, on
/// the file's grid.
#[derive(Debug)]
struct Day {
    times: Vec<i64>,
    grid: CellGrid,
    lat_range: (f64, f64),
    u: Vec<Vec<f32>>,
    v: Vec<Vec<f32>>,
}

type Slot = Arc<OnceLock<std::result::Result<Arc<Day>, String>>>;

/// The product, open: the days the directory lists.
pub struct SeawindsStore {
    fetch: Box<dyn Fetch>,
    days: Vec<(i64, String)>,
    /// Hours since the Unix epoch, sorted: four per listed day.
    times: Vec<i64>,
    /// Days being read or read, with how many of their steps are served.
    held: Mutex<Vec<(i64, Slot, usize)>>,
}

impl std::fmt::Debug for SeawindsStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SeawindsStore")
            .field("days", &self.days.len())
            .finish()
    }
}

impl SeawindsStore {
    /// Reads the directory's index.
    pub fn open(fetch: Box<dyn Fetch>) -> Result<Self> {
        let index = fetch.get(DIRECTORY)?;
        let days = days_listed(&String::from_utf8_lossy(&index));
        if days.is_empty() {
            return Err(ZarrError::Open(format!(
                "{NAME}: the directory lists no day"
            )));
        }
        let times = days
            .iter()
            .flat_map(|&(day, _)| SLOTS.iter().map(move |h| day + h))
            .collect();
        Ok(Self {
            fetch,
            days,
            times,
            held: Mutex::new(Vec::new()),
        })
    }

    /// The day's file, fetched by whichever thread asks first; the others
    /// wait for it.
    fn day(&self, day: i64) -> Result<Arc<Day>> {
        let date = self
            .days
            .iter()
            .find(|(d, _)| *d == day)
            .map(|(_, date)| date.clone())
            .ok_or_else(|| ZarrError::TimeRange(format!("{NAME} lists no file for that day")))?;
        let slot = {
            let mut held = self.held.lock().unwrap_or_else(|e| e.into_inner());
            match held.iter_mut().find(|(d, _, _)| *d == day) {
                Some((_, slot, served)) => {
                    *served += 1;
                    slot.clone()
                }
                None => {
                    if held.len() >= HELD_DAYS {
                        held.remove(0);
                    }
                    let slot = Slot::default();
                    held.push((day, slot.clone(), 1));
                    slot
                }
            }
        };
        let read = slot.get_or_init(|| {
            let url = format!("{DIRECTORY}{PREFIX}{date}{SUFFIX}");
            self.fetch
                .get(&url)
                .and_then(read_day)
                .map(Arc::new)
                .map_err(|err| err.to_string())
        });
        // Every step of the day served: nothing will ask for it again.
        let mut held = self.held.lock().unwrap_or_else(|e| e.into_inner());
        held.retain(|(d, _, served)| *d != day || *served < SLOTS.len());
        read.clone().map_err(ZarrError::Open)
    }
}

/// The day file's axes and components, with fill and missing values NaN
/// and any packing applied.
fn read_day(bytes: Vec<u8>) -> Result<Day> {
    let bad = |err: ve_hdf5::Error| ZarrError::Layout(format!("{NAME}: {err}"));
    let file = ve_hdf5::File::from_bytes(bytes).map_err(bad)?;
    let axis = |name: &str| -> Result<Vec<f32>> {
        let values = file.dataset(name).and_then(|d| d.read_f64()).map_err(bad)?;
        Ok(values.into_iter().map(|v| v as f32).collect())
    };
    let (lat, lon) = (axis("lat")?, axis("lon")?);
    let grid = cell_grid_of(&lat, &lon)?;
    let time = file.dataset("time").map_err(bad)?;
    let units = time
        .attribute("units")
        .and_then(|a| a.text())
        .ok_or_else(|| ZarrError::Layout(format!("{NAME} has no time units")))?;
    let (epoch, unit) = parse_time_units(units)?;
    let times = hours_from_axis(
        epoch.hours_since_unix_epoch(),
        unit,
        &time.read_f64().map_err(bad)?,
    )?;
    let component = |name: &str| -> Result<Vec<Vec<f32>>> {
        let dataset = file.dataset(name).map_err(bad)?;
        let number = |a: &str| dataset.attribute(a).and_then(|a| a.number());
        let fill = number("_FillValue").or_else(|| number("missing_value"));
        let scale = number("scale_factor").unwrap_or(1.0);
        let offset = number("add_offset").unwrap_or(0.0);
        let values = dataset.read_f64().map_err(bad)?;
        if values.len() != times.len() * grid.len() {
            return Err(ZarrError::Layout(format!(
                "{NAME}: {name} holds {} values, not {} times of {}",
                values.len(),
                times.len(),
                grid.len()
            )));
        }
        Ok(values
            .chunks_exact(grid.len())
            .map(|field| {
                field
                    .iter()
                    .map(|&raw| {
                        if Some(raw) == fill || !raw.is_finite() {
                            f32::NAN
                        } else {
                            (raw * scale + offset) as f32
                        }
                    })
                    .collect()
            })
            .collect())
    };
    let (lo, hi) = (f64::from(lat[0]), f64::from(lat[lat.len() - 1]));
    Ok(Day {
        u: component("u_wind")?,
        v: component("v_wind")?,
        times,
        grid,
        lat_range: (lo.min(hi), lo.max(hi)),
    })
}

impl FieldSource for SeawindsStore {
    fn name(&self) -> &'static str {
        NAME
    }

    fn variables(&self) -> Vec<Variable> {
        vec![Variable::Wind10m]
    }

    fn coverage(&self) -> Option<(Utc, Utc)> {
        Some((
            Utc::from_hours_since_unix_epoch(*self.times.first()?),
            Utc::from_hours_since_unix_epoch(*self.times.last()?),
        ))
    }

    fn step_at(&self, time: Utc) -> Option<Step> {
        step_at_hour(&self.times, time)
    }

    fn steps_in_range(&self, start: Utc, end: Utc) -> Result<Vec<Step>> {
        steps_between(&self.times, start, end, NAME)
    }

    /// A time the day's file does not hold reads as a field with no value
    /// anywhere, which the import writes as nothing — as for a day a
    /// Copernicus store lists before it is written.
    fn read_step(&self, step: &Step) -> Result<Vec<Field>> {
        let hour = step.valid_time.hours_since_unix_epoch();
        let day = self.day(hour - hour.rem_euclid(24))?;
        let (u, v) = match day.times.iter().position(|&t| t == hour) {
            Some(k) => (
                regrid_within(day.grid, day.lat_range, &day.u[k]),
                regrid_within(day.grid, day.lat_range, &day.v[k]),
            ),
            None => {
                let empty = vec![f32::NAN; crate::source::POINTS_PER_STEP];
                (empty.clone(), empty)
            }
        };
        Ok(vec![Field {
            variable: Variable::Wind10m,
            u,
            v,
        }])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::{NI, NJ};
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// The directory's index as the server writes it, trimmed.
    const INDEX: &str = r#"<html><body><h1>Index of /data/pub0015/coastwatch/blended/wind/nrt/uvcomp/6hr</h1>
<pre><a href="NBSv02_wind_6hourly_20261001_nrt.nc">NBSv02_wind_6hourly_20261001_nrt.nc</a> 02-Oct-2026 03:12  18M
<a href="NBSv02_wind_6hourly_20260929_nrt.nc">NBSv02_wind_6hourly_20260929_nrt.nc</a> 30-Oct-2026 03:12  18M
<a href="NBSv02_wind_6hourly_2026093_nrt.nc">short</a>
<a href="NBSv02_wind_6hourly_20260930_nrt.nc.md5">NBSv02_wind_6hourly_20260930_nrt.nc.md5</a>
<a href="NBSv02_wind_6hourly_20260930_nrt.nc">NBSv02_wind_6hourly_20260930_nrt.nc</a> 01-Oct-2026 03:12  18M
</pre></body></html>"#;

    fn hours(iso: &str) -> i64 {
        Utc::parse(iso).expect("a time").hours_since_unix_epoch()
    }

    #[test]
    fn the_index_lists_each_day_once_in_order() {
        let days = days_listed(INDEX);
        assert_eq!(
            days,
            [
                (hours("2026-09-29T00:00"), "20260929".to_owned()),
                (hours("2026-09-30T00:00"), "20260930".to_owned()),
                (hours("2026-10-01T00:00"), "20261001".to_owned()),
            ]
        );
    }

    /// A checksum or a partial file beside a day is not the day: listed
    /// alone, the day's download would fail the import.
    #[test]
    fn only_the_day_file_itself_lists_a_day() {
        let index = r#"<a href="NBSv02_wind_6hourly_20260928_nrt.nc.md5">NBSv02_wind_6hourly_20260928_nrt.nc.md5</a>
<a href="NBSv02_wind_6hourly_20260927_nrt.nc.tmp">x</a> NBSv02_wind_6hourly_20260926_nrt.nc
NBSv02_wind_6hourly_20260925_nrt.nc"#;
        let days: Vec<String> = days_listed(index).into_iter().map(|(_, d)| d).collect();
        assert_eq!(days, ["20260925", "20260926"]);
    }

    /// The index, and `fixtures/seawinds`'s one day file for 1 October;
    /// every other day is a 404. Counts the day files fetched.
    struct Server {
        files: AtomicUsize,
    }

    impl Fetch for Server {
        fn get(&self, url: &str) -> Result<Vec<u8>> {
            if url == DIRECTORY {
                return Ok(INDEX.as_bytes().to_vec());
            }
            let name = url.strip_prefix(DIRECTORY).unwrap_or(url);
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/seawinds")
                .join(name);
            self.files.fetch_add(1, Ordering::SeqCst);
            std::fs::read(&path).map_err(|_| ZarrError::Open(format!("{url}: 404")))
        }
    }

    fn store() -> (SeawindsStore, &'static Server) {
        let server: &'static Server = Box::leak(Box::new(Server {
            files: AtomicUsize::new(0),
        }));
        struct Shared(&'static Server);
        impl Fetch for Shared {
            fn get(&self, url: &str) -> Result<Vec<u8>> {
                self.0.get(url)
            }
        }
        (
            SeawindsStore::open(Box::new(Shared(server))).expect("opens"),
            server,
        )
    }

    /// The common grid's value nearest a place, north to south from 90 and
    /// east from 0.
    fn at(field: &[f32], lat: f64, lon: f64) -> f32 {
        let j = ((90.0 - lat) / 0.25).round() as usize;
        let i = (lon.rem_euclid(360.0) / 0.25).round() as usize % NI as usize;
        field[j * NI as usize + i]
    }

    #[test]
    fn four_six_hourly_steps_per_listed_day() {
        let (store, _) = store();
        let steps = store
            .steps_in_range(
                Utc::from_hours_since_unix_epoch(hours("2026-09-29T00:00")),
                Utc::from_hours_since_unix_epoch(hours("2026-10-01T18:00")),
            )
            .expect("steps");
        assert_eq!(steps.len(), 12);
        assert_eq!(
            steps[5].valid_time.hours_since_unix_epoch(),
            hours("2026-09-30T06:00")
        );
    }

    /// The fixture's u is the time's index plus one and v is -2, except
    /// north of 60° N (NaN in the file) and at one cell written as the fill
    /// value.
    #[test]
    fn a_step_reads_its_own_time_from_the_day_file() {
        let (store, _) = store();
        let step = store
            .step_at(Utc::from_hours_since_unix_epoch(hours("2026-10-01T12:00")))
            .expect("listed");
        let field = &store.read_step(&step).expect("reads")[0];
        assert_eq!(field.u.len(), (NI * NJ) as usize);
        assert_eq!(at(&field.u, -30.0, 200.0), 3.0);
        assert_eq!(at(&field.v, -30.0, 200.0), -2.0);
        assert_eq!(at(&field.u, 45.0, -60.0), 3.0, "west of 0 is east of 180");
        assert!(at(&field.u, 75.0, 30.0).is_nan(), "no observation");
        assert!(at(&field.u, 0.5, 10.5).is_nan(), "the fill value");
    }

    #[test]
    fn a_day_is_fetched_once_for_all_its_steps_on_any_thread() {
        let (store, server) = store();
        let day = hours("2026-10-01T00:00");
        std::thread::scope(|scope| {
            for slot in SLOTS {
                let store = &store;
                scope.spawn(move || {
                    let step = store
                        .step_at(Utc::from_hours_since_unix_epoch(day + slot))
                        .expect("listed");
                    let field = &store.read_step(&step).expect("reads")[0];
                    assert_eq!(at(&field.u, 0.0, 100.0), (slot / 6 + 1) as f32);
                });
            }
        });
        assert_eq!(server.files.load(Ordering::SeqCst), 1);
        assert!(
            store.held.lock().expect("lock").is_empty(),
            "a day whose steps were all read is let go"
        );
    }

    #[test]
    fn a_listed_day_that_will_not_download_is_an_error_naming_it() {
        let (store, _) = store();
        let step = store
            .step_at(Utc::from_hours_since_unix_epoch(hours("2026-09-30T06:00")))
            .expect("listed");
        let err = store.read_step(&step).expect_err("404").to_string();
        assert!(err.contains("20260930"), "{err}");
    }
}

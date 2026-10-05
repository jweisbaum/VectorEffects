//! Fields out of a NOAA ERDDAP server (spec.md 4.10).
//!
//! An ERDDAP `griddap` request names a variable, the indices wanted along
//! each axis, and a format; asked for `.nc`, the server answers with exactly
//! that subset as a NetCDF-3 file, which [`crate::netcdf3`] reads. So a
//! product here is a server, a dataset identifier and the names of its
//! variables — nothing is downloaded that is not used.
//!
//! The axes are read once, when the dataset is opened; each time step is
//! then one request for every component at once. A dataset whose grid does
//! not reach the poles — CCMP stops at 78.375 degrees — leaves the common
//! grid's nodes beyond it empty rather than stretching its edge rows over
//! them.
//!
//! **Only named hosts are reached.** A server that answers with a redirect
//! elsewhere is not followed off the host it was asked on; the offline check
//! holds the source to the hosts it names, and it cannot see a redirect.

use reqwest::StatusCode;

use crate::arco::cell_grid_of;
use crate::error::{Result, ZarrError};
use crate::http::{Retry, retrying, transient, with_causes};
use crate::netcdf3::File;
use std::ops::Range;

use crate::regrid::{CellGrid, native_window, place, to_era5_window};
use crate::source::{Field, FieldSource, Step, Variable, Window, step_at_hour, steps_between};
use crate::store::{hours_from_axis, parse_time_units};
use crate::time::Utc;

/// How a product is read off an ERDDAP server.
#[derive(Debug, Clone, Copy)]
pub struct ErddapSpec {
    /// Short name, for logs and messages.
    pub name: &'static str,
    /// The server's ERDDAP root, `https://host/erddap`.
    pub server: &'static str,
    /// The dataset's identifier on that server.
    pub dataset: &'static str,
    /// What the components are of.
    pub variable: Variable,
    /// The eastward component, or the one value of a scalar product.
    pub u: &'static str,
    /// The northward component; `None` for a scalar product.
    pub v: Option<&'static str>,
    /// Every how-many-th latitude and longitude to take: the server strides
    /// a fine grid itself, so a 0.05 degree analysis arrives at 0.25.
    pub stride: usize,
    /// A daily product, filed under each day's midnight (see
    /// [`crate::arco::daily_hours`]).
    pub daily: bool,
    /// The axes between time and latitude, each read at its first index:
    /// OISST's `zlev` is one depth, written as an axis of its own.
    pub extra_axes: usize,
}

/// Something that answers a URL with its bytes: the network, or a test.
pub trait Fetch: Send + Sync {
    /// The body of a `GET`.
    fn get(&self, url: &str) -> Result<Vec<u8>>;
}

/// The network, through a client that stays on the host it is sent to.
#[derive(Debug)]
pub struct Http {
    client: reqwest::blocking::Client,
}

impl Http {
    /// A client that will not follow a redirect to another host.
    pub fn new() -> Result<Self> {
        let policy = reqwest::redirect::Policy::custom(|attempt| {
            let same = attempt
                .previous()
                .first()
                .zip(attempt.url().host_str())
                .is_some_and(|(first, host)| first.host_str() == Some(host));
            if same && attempt.previous().len() < 5 {
                attempt.follow()
            } else {
                attempt.stop()
            }
        });
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(180))
            // A server that does not answer is found out in seconds, not at
            // the operating system's connect timeout: each try would
            // otherwise wait over a minute, and the import holds the whole
            // product list for every try.
            .connect_timeout(std::time::Duration::from_secs(10))
            .redirect(policy)
            .build()
            .map_err(|err| ZarrError::Open(format!("no HTTP client: {err}")))?;
        Ok(Self { client })
    }
}

impl Fetch for Http {
    fn get(&self, url: &str) -> Result<Vec<u8>> {
        retrying(|| {
            let response = self
                .client
                .get(url)
                .send()
                .map_err(|err| Retry::Later(zarrs_storage::StorageError::Other(with_causes(&err))))?;
            let status = response.status();
            if status == StatusCode::OK {
                return response
                    .bytes()
                    .map(|b| b.to_vec())
                    .map_err(|err| Retry::Later(zarrs_storage::StorageError::Other(with_causes(&err))));
            }
            let err = zarrs_storage::StorageError::Other(if status.is_redirection() {
                format!("the server sent the request on to another host, which is not followed ({status})")
            } else {
                format!("the server answered {status}")
            });
            Err(if transient(status) {
                Retry::Later(err)
            } else {
                Retry::No(err)
            })
        })
        .map_err(|err| ZarrError::Open(format!("could not read {url}: {err}")))
    }
}

/// An ERDDAP subscript: one index, or every `stride`-th.
fn subscript(index: Option<usize>, stride: usize) -> String {
    match index {
        Some(i) => format!("%5B{i}%5D"),
        None => format!("%5B0:{}:last%5D", stride.max(1)),
    }
}

/// An ERDDAP subscript over a run of the dataset's strided grid, in the
/// dataset's own indices: every `stride`-th from `range.start`, to the
/// axis's end spelled `last` as the whole-axis request spells it.
fn range_subscript(range: &Range<usize>, stride: usize, n: usize) -> String {
    let stride = stride.max(1);
    let end = if range.end >= n {
        "last".to_owned()
    } else {
        ((range.end - 1) * stride).to_string()
    };
    format!("%5B{}:{stride}:{end}%5D", range.start * stride)
}

/// The requests one step of a window is: for each block of cells, the
/// subscripts after the variable's name, and the block's rows and columns
/// on the strided grid. The global window is one request for every cell, as
/// it always was; a window across the dataset's own seam — 180 degrees on a
/// `LonPM180` dataset, the prime meridian on one that runs 0..360 — is two.
fn step_requests(
    index: usize,
    extra_axes: usize,
    stride: usize,
    grid: CellGrid,
    window: &Window,
) -> Vec<(String, Range<usize>, Range<usize>)> {
    let native = native_window(grid, window);
    native
        .columns
        .iter()
        .map(|columns| {
            let mut indices = subscript(Some(index), 1);
            for _ in 0..extra_axes {
                indices.push_str(&subscript(Some(0), 1));
            }
            indices.push_str(&range_subscript(&native.rows, stride, grid.nlat));
            indices.push_str(&range_subscript(columns, stride, grid.nlon));
            (indices, native.rows.clone(), columns.clone())
        })
        .collect()
}

/// An open handle on one ERDDAP dataset.
pub struct ErddapStore {
    spec: ErddapSpec,
    fetch: Box<dyn Fetch>,
    grid: CellGrid,
    /// Southernmost and northernmost cell centres.
    lat_range: (f64, f64),
    /// Hours since the Unix epoch, sorted.
    times: Vec<i64>,
    /// What every read asks the server for (M102).
    window: Window,
}

impl std::fmt::Debug for ErddapStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ErddapStore")
            .field("dataset", &self.spec.dataset)
            .field("steps", &self.times.len())
            .finish()
    }
}

impl ErddapStore {
    /// The URL of a `.nc` request on the dataset.
    fn url(&self, query: &str) -> String {
        format!(
            "{}/griddap/{}.nc?{query}",
            self.spec.server, self.spec.dataset
        )
    }

    /// Opens the dataset: one request for its axes.
    pub fn open(spec: ErddapSpec, fetch: Box<dyn Fetch>) -> Result<Self> {
        let every = subscript(None, spec.stride);
        let url = format!(
            "{}/griddap/{}.nc?time,latitude{every},longitude{every}",
            spec.server, spec.dataset
        );
        let bytes = fetch.get(&url)?;
        let file = File::parse(&bytes)?;
        let time = file.read_f64("time")?;
        let units = file
            .variable("time")
            .and_then(|t| t.text("units"))
            .ok_or_else(|| ZarrError::Layout(format!("{} has no time units", spec.name)))?
            .to_owned();
        let (epoch, unit) = parse_time_units(&units)?;
        let times = hours_from_axis(epoch.hours_since_unix_epoch(), unit, &time)?;
        let times = if spec.daily {
            crate::arco::daily_hours(&times)?
        } else {
            times
        };
        let lat: Vec<f32> = file
            .read_f64("latitude")?
            .iter()
            .map(|x| *x as f32)
            .collect();
        let lon: Vec<f32> = file
            .read_f64("longitude")?
            .iter()
            .map(|x| *x as f32)
            .collect();
        let grid = cell_grid_of(&lat, &lon)?;
        let (lo, hi) = (f64::from(lat[0]), f64::from(lat[lat.len() - 1]));
        Ok(Self {
            spec,
            fetch,
            grid,
            lat_range: (lo.min(hi), lo.max(hi)),
            times,
            window: Window::global(),
        })
    }

    /// The components at one step, on the dataset's own grid, NaN where it
    /// has no value.
    fn read_native(&self, index: usize) -> Result<Vec<Vec<f32>>> {
        let names = self.components();
        let requests = step_requests(
            index,
            self.spec.extra_axes,
            self.spec.stride,
            self.grid,
            &self.window,
        );
        let whole =
            requests.len() == 1 && requests[0].1.len() * requests[0].2.len() == self.grid.len();
        let mut out: Vec<Vec<f32>> = names
            .iter()
            .map(|_| {
                if whole {
                    Vec::new()
                } else {
                    vec![f32::NAN; self.grid.len()]
                }
            })
            .collect();
        for (indices, rows, columns) in requests {
            let expected = rows.len() * columns.len();
            let query = names
                .iter()
                .map(|name| format!("{name}{indices}"))
                .collect::<Vec<_>>()
                .join(",");
            let bytes = self.fetch.get(&self.url(&query))?;
            let file = File::parse(&bytes)?;
            for (name, buffer) in names.iter().zip(&mut out) {
                let block = self.unpacked(&file, name, expected)?;
                if whole {
                    *buffer = block;
                } else {
                    place(self.grid, buffer, rows.clone(), columns.clone(), &block);
                }
            }
        }
        Ok(out)
    }

    /// One variable of a response in physical units, NaN where it has none.
    fn unpacked(&self, file: &File, name: &str, expected: usize) -> Result<Vec<f32>> {
        let variable = file
            .variable(name)
            .ok_or_else(|| ZarrError::Layout(format!("{} sent no {name}", self.spec.name)))?;
        let fill = variable
            .number("_FillValue")
            .or_else(|| variable.number("missing_value"));
        let scale = variable.number("scale_factor").unwrap_or(1.0);
        let offset = variable.number("add_offset").unwrap_or(0.0);
        let values = file.read_f64(name)?;
        if values.len() != expected {
            return Err(ZarrError::Layout(format!(
                "{} sent {} values of {name}, expected {expected}",
                self.spec.name,
                values.len(),
            )));
        }
        Ok(values
            .into_iter()
            .map(|raw| {
                if Some(raw) == fill || !raw.is_finite() {
                    f32::NAN
                } else {
                    (raw * scale + offset) as f32
                }
            })
            .collect())
    }

    /// The variables read at each step: both components, or the one value.
    fn components(&self) -> Vec<&'static str> {
        match self.spec.v {
            Some(v) => vec![self.spec.u, v],
            None => vec![self.spec.u],
        }
    }

    fn regrid(&self, values: &[f32]) -> Vec<f32> {
        regrid_window(self.grid, self.lat_range, values, &self.window)
    }
}

/// Onto the common grid, empty beyond the latitudes a dataset reaches:
/// `lat_range` is its southernmost and northernmost cell centres.
pub(crate) fn regrid_within(grid: CellGrid, lat_range: (f64, f64), values: &[f32]) -> Vec<f32> {
    regrid_window(grid, lat_range, values, &Window::global())
}

/// [`regrid_within`] on only the nodes of `window`.
fn regrid_window(
    grid: CellGrid,
    lat_range: (f64, f64),
    values: &[f32],
    window: &Window,
) -> Vec<f32> {
    let mut out = to_era5_window(grid, values, window);
    let half = grid.dlat.abs() / 2.0;
    let ni = window.ni as usize;
    for (k, j) in window.rows().enumerate() {
        let lat = 90.0 - j as f64 * 0.25;
        if lat > lat_range.1 + half + 1e-9 || lat < lat_range.0 - half - 1e-9 {
            out[k * ni..(k + 1) * ni].fill(f32::NAN);
        }
    }
    out
}

impl FieldSource for ErddapStore {
    fn name(&self) -> &'static str {
        self.spec.name
    }

    fn variables(&self) -> Vec<Variable> {
        vec![self.spec.variable]
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
        steps_between(&self.times, start, end, self.spec.name)
    }

    fn read_step(&self, step: &Step) -> Result<Vec<Field>> {
        let mut native = self.read_native(step.index as usize)?.into_iter();
        let u = native.next().unwrap_or_default();
        let v = native.next();
        Ok(vec![Field {
            variable: self.spec.variable,
            u: self.regrid(&u),
            v: v.map(|v| self.regrid(&v)).unwrap_or_default(),
        }])
    }

    /// The server answers index subscripts with only those cells, so a
    /// regional fetch asks for its window (M102).
    fn set_window(&mut self, window: Window) {
        self.window = window;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// The smallest NetCDF-3 file the server could send: named 1-D axes and
    /// float variables over `(time, latitude, longitude)`.
    fn netcdf(axes: &[(&str, &[f64], &str)], fields: &[(&str, &[f32], f32)]) -> Vec<u8> {
        fn name(out: &mut Vec<u8>, s: &str) {
            out.extend((s.len() as u32).to_be_bytes());
            out.extend(s.as_bytes());
            out.resize(out.len() + (4 - s.len() % 4) % 4, 0);
        }
        let mut header = b"CDF\x01".to_vec();
        header.extend(0u32.to_be_bytes());
        header.extend(0x0Au32.to_be_bytes());
        header.extend((axes.len() as u32).to_be_bytes());
        for (n, values, _) in axes {
            name(&mut header, n);
            header.extend((values.len() as u32).to_be_bytes());
        }
        header.extend([0u8; 8]);
        header.extend(0x0Bu32.to_be_bytes());
        header.extend(((axes.len() + fields.len()) as u32).to_be_bytes());
        // Data begins after a header whose length is known once written, so
        // the offsets are patched in afterwards.
        let mut patches = Vec::new();
        let mut data: Vec<Vec<u8>> = Vec::new();
        for (i, (n, values, units)) in axes.iter().enumerate() {
            name(&mut header, n);
            header.extend(1u32.to_be_bytes());
            header.extend((i as u32).to_be_bytes());
            header.extend(0x0Cu32.to_be_bytes());
            header.extend(1u32.to_be_bytes());
            name(&mut header, "units");
            header.extend(2u32.to_be_bytes());
            name(&mut header, units);
            header.extend(6u32.to_be_bytes());
            header.extend(((values.len() * 8) as u32).to_be_bytes());
            patches.push(header.len());
            header.extend(0u32.to_be_bytes());
            data.push(values.iter().flat_map(|v| v.to_be_bytes()).collect());
        }
        for (n, values, fill) in fields {
            name(&mut header, n);
            header.extend(3u32.to_be_bytes());
            for d in 0..3u32 {
                header.extend(d.to_be_bytes());
            }
            header.extend(0x0Cu32.to_be_bytes());
            header.extend(1u32.to_be_bytes());
            name(&mut header, "_FillValue");
            header.extend(5u32.to_be_bytes());
            header.extend(1u32.to_be_bytes());
            header.extend(fill.to_be_bytes());
            header.extend(5u32.to_be_bytes());
            header.extend(((values.len() * 4) as u32).to_be_bytes());
            patches.push(header.len());
            header.extend(0u32.to_be_bytes());
            data.push(values.iter().flat_map(|v| v.to_be_bytes()).collect());
        }
        let mut at = header.len();
        for (patch, block) in patches.iter().zip(&data) {
            header[*patch..*patch + 4].copy_from_slice(&(at as u32).to_be_bytes());
            at += block.len();
        }
        for block in data {
            header.extend(block);
        }
        header
    }

    /// Answers by URL, and remembers what it was asked.
    struct Canned {
        answers: Vec<(String, Vec<u8>)>,
        asked: Mutex<Vec<String>>,
    }

    impl Fetch for Canned {
        fn get(&self, url: &str) -> Result<Vec<u8>> {
            self.asked.lock().unwrap().push(url.to_owned());
            self.answers
                .iter()
                .find(|(prefix, _)| url.contains(prefix.as_str()))
                .map(|(_, bytes)| bytes.clone())
                .ok_or_else(|| ZarrError::Open(format!("nothing canned for {url}")))
        }
    }

    const SPEC: ErddapSpec = ErddapSpec {
        name: "test",
        server: "https://example.invalid/erddap",
        dataset: "winds",
        variable: Variable::Wind10m,
        u: "uwnd",
        v: Some("vwnd"),
        extra_axes: 0,
        stride: 1,
        daily: false,
    };

    /// Cells 30 degrees tall centred on 30 S, 0 and 30 N — so the data stops
    /// at 45 degrees either way — and 90 wide on 0..360 longitude: the shape
    /// CCMP has, at a size a test can hold.
    fn coarse() -> (Vec<f64>, Vec<f64>) {
        (vec![-30.0, 0.0, 30.0], vec![45.0, 135.0, 225.0, 315.0])
    }

    fn store(times: &[f64], fields: &[u8]) -> (ErddapStore, std::sync::Arc<Vec<String>>) {
        let (lat, lon) = coarse();
        let axes = netcdf(
            &[
                ("time", times, "seconds since 1970-01-01T00:00:00Z"),
                ("latitude", &lat, "degrees_north"),
                ("longitude", &lon, "degrees_east"),
            ],
            &[],
        );
        let canned = Canned {
            answers: vec![
                (".nc?time,".to_owned(), axes),
                ("uwnd".to_owned(), fields.to_vec()),
            ],
            asked: Mutex::new(Vec::new()),
        };
        let store = ErddapStore::open(SPEC, Box::new(canned)).expect("opens");
        (store, std::sync::Arc::new(Vec::new()))
    }

    /// Opening reads the axes once and turns seconds since 1970 into hours:
    /// 2026-10-01T00Z and six hours later.
    #[test]
    fn the_axes_become_hours_and_a_grid() {
        let t0 = 20_727.0 * 86_400.0;
        let (store, _) = store(&[t0, t0 + 6.0 * 3600.0], &[]);
        assert_eq!(store.times, vec![20_727 * 24, 20_727 * 24 + 6]);
        assert_eq!(store.grid.nlon, 4);
        assert_eq!(store.lat_range, (-30.0, 30.0));
        let (first, last) = store.coverage().expect("coverage");
        assert_eq!((first.day, first.hour, last.hour), (1, 0, 6));
    }

    /// A step is one request for both components at that time index, the
    /// fill comes back missing, and the common grid is empty beyond the
    /// latitudes the dataset reaches — not its edge row stretched to the pole.
    #[test]
    fn a_step_is_both_components_with_the_fill_missing_and_no_poles() {
        let t0 = 20_727.0 * 86_400.0;
        let (lat, lon) = coarse();
        let fill = -9999.0_f32;
        // Row by row from 30 S; the cell at 30 S, 225 E is the fill.
        let mut u = [3.0_f32; 12];
        u[2] = fill;
        let v = [0.0_f32; 12];
        let fields = netcdf(
            &[
                ("time", &[t0], "seconds since 1970-01-01T00:00:00Z"),
                ("latitude", &lat, "degrees_north"),
                ("longitude", &lon, "degrees_east"),
            ],
            &[("uwnd", &u, fill), ("vwnd", &v, fill)],
        );
        let (store, _) = store(&[t0], &fields);
        let step = store.steps_in_range(
            Utc::from_hours_since_unix_epoch(20_727 * 24),
            Utc::from_hours_since_unix_epoch(20_727 * 24),
        );
        let field = &store.read_step(&step.expect("a step")[0]).expect("read")[0];
        let ni = 1440;
        let at = |lat: f64, lon: f64| {
            let j = ((90.0 - lat) / 0.25).round() as usize;
            let i = (lon / 0.25).round() as usize;
            field.u[j * ni + i]
        };
        assert_eq!(at(0.0, 90.0), 3.0);
        // The data's edge is 45 degrees: up to it there is wind, past it
        // nothing — the edge rows are not stretched to the poles.
        assert_eq!(at(45.0, 90.0), 3.0);
        assert!(at(45.25, 90.0).is_nan());
        assert!(at(90.0, 0.0).is_nan());
        assert!(at(-90.0, 0.0).is_nan());
        // A node at the filled cell's very centre is missing; one between it
        // and its neighbours takes what the neighbours have.
        assert!(at(-30.0, 225.0).is_nan());
        assert_eq!(at(-30.0, 180.0), 3.0);
        assert!(field.v.iter().filter(|x| x.is_finite()).all(|x| *x == 0.0));
    }

    /// OISST's `_LonPM180` copy runs from -179.875: a window from 160 E to
    /// 160 W crosses its seam, and is asked for as two requests — from
    /// 160 E to the end of the row, and from the start of the row to 160 W.
    /// Geo-Polar's grid is a twentieth of a degree read at every fifth
    /// point, so its subscripts count in the dataset's own indices.
    #[test]
    fn erddap_subscript_splits_at_its_seam() {
        let pm180 = CellGrid {
            lat0: -89.875,
            dlat: 0.25,
            nlat: 720,
            lon0: -179.875,
            dlon: 0.25,
            nlon: 1440,
        };
        let across = Window {
            i0: 640,
            ni: 161,
            j0: 320,
            nj: 81,
        };
        let requests = step_requests(3, 1, 1, pm180, &across);
        let spelled: Vec<&str> = requests.iter().map(|(s, _, _)| s.as_str()).collect();
        assert_eq!(
            spelled,
            [
                "%5B3%5D%5B0%5D%5B318:1:401%5D%5B1358:1:last%5D",
                "%5B3%5D%5B0%5D%5B318:1:401%5D%5B0:1:81%5D",
            ]
        );
        // The same window on a 0.05 degree grid strided by five.
        let geo_polar = CellGrid {
            lat0: -89.975,
            dlat: 0.25,
            nlat: 720,
            lon0: -179.975,
            dlon: 0.25,
            nlon: 1440,
        };
        let requests = step_requests(3, 0, 5, geo_polar, &across);
        assert_eq!(requests.len(), 2);
        assert!(
            requests[0].0.ends_with("%5B6790:5:last%5D"),
            "{}",
            requests[0].0
        );
        assert!(
            requests[1].0.ends_with("%5B0:5:405%5D"),
            "{}",
            requests[1].0
        );
        // A window inside the row is one request, and the whole grid is the
        // request it always was.
        let middle = Window {
            i0: 1360,
            ni: 161,
            j0: 120,
            nj: 81,
        };
        assert_eq!(step_requests(3, 1, 1, pm180, &middle).len(), 1);
        let whole = step_requests(3, 0, 1, pm180, &Window::global());
        assert_eq!(whole.len(), 1);
        assert_eq!(whole[0].0, "%5B3%5D%5B0:1:last%5D%5B0:1:last%5D");
    }

    /// A windowed step reads back the window of what the whole step reads:
    /// the dataset is asked twice across its seam and the two blocks land
    /// where they belong.
    #[test]
    fn a_windowed_step_is_the_window_of_the_whole_step() {
        let t0 = 20_727.0 * 86_400.0;
        // 1-degree cells, -179.5 .. 179.5 and -89.5 .. 89.5, a value per cell.
        let lat: Vec<f64> = (0..180).map(|j| -89.5 + j as f64).collect();
        let lon: Vec<f64> = (0..360).map(|i| -179.5 + i as f64).collect();
        let u: Vec<f32> = (0..180 * 360).map(|k| (k % 997) as f32).collect();
        let v: Vec<f32> = (0..180 * 360).map(|k| (k % 13) as f32).collect();
        let axes = netcdf(
            &[
                ("time", &[t0], "seconds since 1970-01-01T00:00:00Z"),
                ("latitude", &lat, "degrees_north"),
                ("longitude", &lon, "degrees_east"),
            ],
            &[],
        );
        // The server's answer to any subscript, cut from the whole field.
        struct Server {
            axes: Vec<u8>,
            lat: Vec<f64>,
            lon: Vec<f64>,
            u: Vec<f32>,
            v: Vec<f32>,
            asked: Mutex<Vec<String>>,
        }
        impl Fetch for Server {
            fn get(&self, url: &str) -> Result<Vec<u8>> {
                self.asked.lock().unwrap().push(url.to_owned());
                if url.contains(".nc?time,") {
                    return Ok(self.axes.clone());
                }
                let parse = |s: &str, n: usize| -> Range<usize> {
                    let parts: Vec<&str> = s.split(':').collect();
                    let a: usize = parts[0].parse().unwrap();
                    let b = if parts[2] == "last" {
                        n - 1
                    } else {
                        parts[2].parse().unwrap()
                    };
                    a..b + 1
                };
                let subs: Vec<&str> = url
                    .split("uwnd")
                    .nth(1)
                    .unwrap()
                    .split(',')
                    .next()
                    .unwrap()
                    .split("%5B")
                    .filter(|s| !s.is_empty())
                    .map(|s| s.trim_end_matches("%5D"))
                    .collect();
                let rows = parse(subs[1], 180);
                let cols = parse(subs[2], 360);
                let cut = |f: &[f32]| -> Vec<f32> {
                    rows.clone()
                        .flat_map(|r| cols.clone().map(move |c| f[r * 360 + c]))
                        .collect()
                };
                let (u, v) = (cut(&self.u), cut(&self.v));
                let lat: Vec<f64> = self.lat[rows.clone()].to_vec();
                let lon: Vec<f64> = self.lon[cols.clone()].to_vec();
                Ok(netcdf(
                    &[
                        (
                            "time",
                            &[20_727.0 * 86_400.0],
                            "seconds since 1970-01-01T00:00:00Z",
                        ),
                        ("latitude", &lat, "degrees_north"),
                        ("longitude", &lon, "degrees_east"),
                    ],
                    &[("uwnd", &u, -9999.0), ("vwnd", &v, -9999.0)],
                ))
            }
        }
        let server = Server {
            axes,
            lat,
            lon,
            u,
            v,
            asked: Mutex::new(Vec::new()),
        };
        let mut store = ErddapStore::open(SPEC, Box::new(server)).expect("opens");
        let step = store
            .step_at(Utc::from_hours_since_unix_epoch(20_727 * 24))
            .unwrap();
        let whole = store.read_step(&step).expect("whole")[0].clone();
        let across = Window {
            i0: 640,
            ni: 161,
            j0: 320,
            nj: 81,
        };
        store.set_window(across);
        let windowed = store.read_step(&step).expect("window")[0].clone();
        let cropped = whole.cropped(&across);
        assert_eq!(windowed.u, cropped.u);
        assert_eq!(windowed.v, cropped.v);
    }

    #[test]
    fn a_step_url_names_the_index_and_both_components() {
        let mut indices = subscript(Some(42), 1);
        indices.push_str(&subscript(None, 5));
        assert_eq!(indices, "%5B42%5D%5B0:5:last%5D");
    }
}

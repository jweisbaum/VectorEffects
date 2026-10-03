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
use crate::regrid::{CellGrid, to_era5_grid};
use crate::source::{Field, FieldSource, NJ, Step, Variable, step_at_hour, steps_between};
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
    /// The eastward and northward components.
    pub u: &'static str,
    /// .
    pub v: &'static str,
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

/// An ERDDAP subscript: one index, or every index.
fn subscript(index: Option<usize>) -> String {
    match index {
        Some(i) => format!("%5B{i}%5D"),
        None => "%5B0:1:last%5D".to_owned(),
    }
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
        let url = format!(
            "{}/griddap/{}.nc?time,latitude,longitude",
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
        })
    }

    /// The components at one step, on the dataset's own grid, NaN where it
    /// has no value.
    fn read_native(&self, index: usize) -> Result<Vec<Vec<f32>>> {
        let mut indices = subscript(Some(index));
        for _ in 0..self.spec.extra_axes {
            indices.push_str(&subscript(Some(0)));
        }
        indices.push_str(&subscript(None));
        indices.push_str(&subscript(None));
        let query = [self.spec.u, self.spec.v]
            .iter()
            .map(|name| format!("{name}{indices}"))
            .collect::<Vec<_>>()
            .join(",");
        let bytes = self.fetch.get(&self.url(&query))?;
        let file = File::parse(&bytes)?;
        [self.spec.u, self.spec.v]
            .iter()
            .map(|name| {
                let variable = file.variable(name).ok_or_else(|| {
                    ZarrError::Layout(format!("{} sent no {name}", self.spec.name))
                })?;
                let fill = variable
                    .number("_FillValue")
                    .or_else(|| variable.number("missing_value"));
                let scale = variable.number("scale_factor").unwrap_or(1.0);
                let offset = variable.number("add_offset").unwrap_or(0.0);
                let values = file.read_f64(name)?;
                if values.len() != self.grid.len() {
                    return Err(ZarrError::Layout(format!(
                        "{} sent {} values of {name}, expected {}",
                        self.spec.name,
                        values.len(),
                        self.grid.len()
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
            })
            .collect()
    }

    /// Onto the common grid, empty beyond the latitudes the dataset reaches.
    fn regrid(&self, values: &[f32]) -> Vec<f32> {
        let mut out = to_era5_grid(self.grid, values);
        let half = self.grid.dlat.abs() / 2.0;
        let ni = out.len() / NJ as usize;
        for j in 0..NJ as usize {
            let lat = 90.0 - j as f64 * 0.25;
            if lat > self.lat_range.1 + half + 1e-9 || lat < self.lat_range.0 - half - 1e-9 {
                out[j * ni..(j + 1) * ni].fill(f32::NAN);
            }
        }
        out
    }
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
        let mut native = self.read_native(step.index as usize)?;
        let v = native.pop().unwrap_or_default();
        let u = native.pop().unwrap_or_default();
        Ok(vec![Field {
            variable: self.spec.variable,
            u: self.regrid(&u),
            v: self.regrid(&v),
        }])
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
        v: "vwnd",
        extra_axes: 0,
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
                ("time,latitude,longitude".to_owned(), axes),
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

    #[test]
    fn a_step_url_names_the_index_and_both_components() {
        let mut indices = subscript(Some(42));
        indices.push_str(&subscript(None));
        assert_eq!(indices, "%5B42%5D%5B0:1:last%5D");
    }
}

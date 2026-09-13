//! Total surface current out of the Copernicus Marine GlobCurrent stores.
//!
//! GlobCurrent (`MULTIOBS_GLO_PHY_MYNRT_015_003`) is an observation-derived
//! product: geostrophic current from altimetry, plus a wind-driven Ekman
//! component, plus the FES2022 barotropic tide. Its `uo`/`vo` arrays are the
//! sum of the three, which is what a passage planner means by "current".
//!
//! Two datasets are read together. The reprocessed multi-year one (`my`)
//! runs from 1993 to a few months ago; the near-real-time one (`nrt`) runs
//! from 2022 to about a day ago. Where both have an hour, `my` wins, since
//! it was built from final rather than provisional altimetry.
//!
//! # What has to change on the way to a GRIB
//!
//! - Values are `int16` scaled by 0.001, with 32767 over land and under sea
//!   ice. They become metres per second with NaN where masked.
//! - Arrays are `(time, elevation, latitude, longitude)`; elevation holds
//!   `[-15, 0]` and the surface is whichever index is 0.
//! - The grid is cell-centred, south to north from -89.875 and east from
//!   -179.875, so every field is regridded onto the ERA5 grid; see
//!   [`crate::regrid`].
//! - The S3 endpoint answers a missing key with 403, not 404, which
//!   [`crate::store::open_array`] accounts for.

use crate::error::{Result, ZarrError};
use crate::parallel::try_join;
use crate::regrid::{CellGrid, to_era5_grid};
use crate::source::{Field, FieldSource, Step, Variable, step_at_hour, steps_between};
use crate::store::{
    ReadArray, check_axis, open_array, open_http, read_axis_f32, read_err, read_time_axis,
};
use crate::time::Utc;

/// The reprocessed multi-year hourly dataset, 1993 onwards.
pub const DEFAULT_MY_URL: &str = "https://s3.waw3-1.cloudferro.com/mdl-arco-time-037/arco/MULTIOBS_GLO_PHY_MYNRT_015_003/cmems_obs-mob_glo_phy-cur_my_0.25deg_PT1H-i_202411/timeChunked.zarr";

/// The near-real-time hourly dataset, 2022 onwards.
pub const DEFAULT_NRT_URL: &str = "https://s3.waw3-1.cloudferro.com/mdl-arco-time-039/arco/MULTIOBS_GLO_PHY_MYNRT_015_003/cmems_obs-mob_glo_phy-cur_nrt_0.25deg_PT1H-i_202411/timeChunked.zarr";

const U_PATH: &str = "/uo";
const V_PATH: &str = "/vo";

/// The grid the store publishes on.
const GRID: CellGrid = CellGrid::GLOBCURRENT;

/// One of the two datasets.
struct Dataset {
    label: &'static str,
    u: ReadArray,
    v: ReadArray,
    /// Index along the elevation axis for the surface.
    surface: u64,
    scale: f32,
    offset: f32,
    fill: i16,
    /// Hours since the Unix epoch, sorted.
    times: Vec<i64>,
}

/// Which dataset a merged step comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Which {
    My,
    Nrt,
}

/// An open handle on both GlobCurrent datasets.
pub struct GlobCurrentStore {
    my: Dataset,
    nrt: Dataset,
    /// Every hour either dataset has, sorted, with where to read it.
    merged: Vec<(i64, Which, u64)>,
    /// Just the hours of `merged`, for searching.
    hours: Vec<i64>,
}

impl std::fmt::Debug for GlobCurrentStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GlobCurrentStore")
            .field("my_steps", &self.my.times.len())
            .field("nrt_steps", &self.nrt.times.len())
            .field(
                "coverage",
                &self.coverage().map(|(a, b)| (a.to_iso(), b.to_iso())),
            )
            .finish()
    }
}

/// Reads a numeric attribute, with a default.
fn attr_f32(array: &ReadArray, key: &str, default: f32) -> Result<f32> {
    match array.attributes().get(key) {
        None => Ok(default),
        Some(v) => v
            .as_f64()
            .map(|x| x as f32)
            .ok_or_else(|| ZarrError::Layout(format!("attribute {key} is not a number: {v}"))),
    }
}

fn open_dataset(url: &str, label: &'static str) -> Result<Dataset> {
    let store = open_http(url)?;
    let (u, v) = try_join(|| open_array(&store, U_PATH), || open_array(&store, V_PATH))?;

    for (path, array) in [(U_PATH, &u), (V_PATH, &v)] {
        match array.shape() {
            [_, _, nj, ni] if *nj as usize == GRID.nlat && *ni as usize == GRID.nlon => {}
            other => {
                return Err(ZarrError::Layout(format!(
                    "{label} {path} has shape {other:?}, expected [time, elevation, {}, {}]",
                    GRID.nlat, GRID.nlon
                )));
            }
        }
        let dtype = format!("{:?}", array.data_type()).to_ascii_lowercase();
        if !dtype.contains("int16") {
            return Err(ZarrError::Layout(format!(
                "{label} {path} is stored as {dtype}; this reader expects scaled int16"
            )));
        }
    }
    if u.shape() != v.shape() {
        return Err(ZarrError::Layout(format!(
            "{label} u and v have different shapes: {:?} vs {:?}",
            u.shape(),
            v.shape()
        )));
    }

    let ((lat, lon), (elevation, times)) = try_join(
        || {
            try_join(
                || read_axis_f32(&store, "/latitude", "the latitude coordinate"),
                || read_axis_f32(&store, "/longitude", "the longitude coordinate"),
            )
        },
        || {
            try_join(
                || read_axis_f32(&store, "/elevation", "the elevation coordinate"),
                || read_time_axis(&store, "/time", u.shape()[0]),
            )
        },
    )?;
    check_axis(
        "latitude",
        &lat,
        GRID.nlat,
        GRID.lat0 as f32,
        GRID.dlat as f32,
    )?;
    check_axis(
        "longitude",
        &lon,
        GRID.nlon,
        GRID.lon0 as f32,
        GRID.dlon as f32,
    )?;

    let surface = elevation.iter().position(|&e| e == 0.0).ok_or_else(|| {
        ZarrError::Layout(format!(
            "{label} has no surface level; elevation axis is {elevation:?}"
        ))
    })? as u64;
    if u.shape()[1] != elevation.len() as u64 {
        return Err(ZarrError::Layout(format!(
            "{label} elevation axis has {} entries but the arrays have {}",
            elevation.len(),
            u.shape()[1]
        )));
    }

    let scale = attr_f32(&u, "scale_factor", 1.0)?;
    let offset = attr_f32(&u, "add_offset", 0.0)?;
    if (
        attr_f32(&v, "scale_factor", 1.0)?,
        attr_f32(&v, "add_offset", 0.0)?,
    ) != (scale, offset)
    {
        return Err(ZarrError::Layout(format!(
            "{label} u and v are scaled differently"
        )));
    }
    let fill = match u.fill_value().as_ne_bytes() {
        [a, b] => i16::from_ne_bytes([*a, *b]),
        other => {
            return Err(ZarrError::Layout(format!(
                "{label} fill value is {} bytes, expected an int16",
                other.len()
            )));
        }
    };

    Ok(Dataset {
        label,
        u,
        v,
        surface,
        scale,
        offset,
        fill,
        times,
    })
}

impl Dataset {
    /// Reads one component at one step, on the store's own grid, in metres
    /// per second with NaN where masked.
    fn read_native(&self, array: &ReadArray, index: u64, name: &str) -> Result<Vec<f32>> {
        let raw = array
            .retrieve_chunk::<Vec<i16>>(&[index, self.surface, 0, 0])
            .map_err(read_err(&format!(
                "the {} {name} current chunk",
                self.label
            )))?;
        if raw.len() != GRID.len() {
            return Err(ZarrError::Layout(format!(
                "the {} {name} chunk at step {index} holds {} values, expected {}",
                self.label,
                raw.len(),
                GRID.len()
            )));
        }
        Ok(raw
            .into_iter()
            .map(|r| {
                if r == self.fill {
                    f32::NAN
                } else {
                    f32::from(r) * self.scale + self.offset
                }
            })
            .collect())
    }
}

impl GlobCurrentStore {
    /// Opens both datasets and validates their layout.
    pub fn open(my_url: &str, nrt_url: &str) -> Result<Self> {
        let (my, nrt) = try_join(
            || open_dataset(my_url, "GlobCurrent my"),
            || open_dataset(nrt_url, "GlobCurrent nrt"),
        )?;

        // Merge the two hour lists. `my` wins where both have an hour.
        let mut merged: Vec<(i64, Which, u64)> = my
            .times
            .iter()
            .enumerate()
            .map(|(i, &h)| (h, Which::My, i as u64))
            .collect();
        let my_last = my.times.last().copied().unwrap_or(i64::MIN);
        merged.extend(
            nrt.times
                .iter()
                .enumerate()
                .filter(|&(_, &h)| h > my_last)
                .map(|(i, &h)| (h, Which::Nrt, i as u64)),
        );
        merged.sort_by_key(|&(h, _, _)| h);
        merged.dedup_by_key(|e| e.0);
        let hours = merged.iter().map(|&(h, _, _)| h).collect();

        Ok(Self {
            my,
            nrt,
            merged,
            hours,
        })
    }

    /// Reads the total current on the store's own grid, before regridding.
    /// Exposed for verification against an independent regrid.
    pub fn read_native(&self, step: &Step) -> Result<(Vec<f32>, Vec<f32>)> {
        let &(_, which, index) = self.merged.get(step.index as usize).ok_or_else(|| {
            ZarrError::TimeRange(format!("step {} is past the merged axis", step.index))
        })?;
        let ds = match which {
            Which::My => &self.my,
            Which::Nrt => &self.nrt,
        };
        try_join(
            || ds.read_native(&ds.u, index, "u"),
            || ds.read_native(&ds.v, index, "v"),
        )
    }

    /// The store's own grid.
    pub fn native_grid() -> CellGrid {
        GRID
    }
}

impl FieldSource for GlobCurrentStore {
    fn name(&self) -> &'static str {
        "GlobCurrent"
    }

    fn variables(&self) -> Vec<Variable> {
        vec![Variable::SurfaceCurrent]
    }

    fn coverage(&self) -> Option<(Utc, Utc)> {
        Some((
            Utc::from_hours_since_unix_epoch(*self.hours.first()?),
            Utc::from_hours_since_unix_epoch(*self.hours.last()?),
        ))
    }

    fn provisional_from(&self) -> Option<Utc> {
        // Everything past the reprocessed dataset's last hour is near-real-time,
        // whether or not `my` also has gaps of its own earlier on.
        let last_my = *self.my.times.last()?;
        let first_nrt = self.hours.iter().copied().find(|&h| h > last_my)?;
        Some(Utc::from_hours_since_unix_epoch(first_nrt))
    }

    fn step_at(&self, time: Utc) -> Option<Step> {
        step_at_hour(&self.hours, time)
    }

    fn steps_in_range(&self, start: Utc, end: Utc) -> Result<Vec<Step>> {
        steps_between(&self.hours, start, end, "GlobCurrent")
    }

    fn read_step(&self, step: &Step) -> Result<Vec<Field>> {
        let (u, v) = self.read_native(step)?;
        let u = to_era5_grid(GRID, &u);
        let v = to_era5_grid(GRID, &v);
        // A field with nothing in it is an unwritten chunk, not an ocean.
        if u.iter().all(|x| x.is_nan()) || v.iter().all(|x| x.is_nan()) {
            return Err(ZarrError::Layout(format!(
                "the GlobCurrent chunk at {} is entirely masked; it is probably not written yet",
                step.valid_time.to_iso()
            )));
        }
        Ok(vec![Field {
            variable: Variable::SurfaceCurrent,
            u,
            v,
        }])
    }
}

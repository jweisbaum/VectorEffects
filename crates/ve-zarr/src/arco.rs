//! A vector field out of any Copernicus Marine time-chunked store.
//!
//! The Marine Data Store publishes every product the same way: scaled
//! integers on a cell-centred global grid, `(time, latitude, longitude)` or
//! `(time, depth, latitude, longitude)`, with the scale, the offset, the
//! fill value and the time axis's epoch in the store's own metadata. So one
//! reader, told which two arrays are `u` and `v`, reads all of them; what
//! differs between products is in [`ArcoSpec`] and in the store.
//!
//! [`crate::globcurrent`] is the particular reader this generalises. It
//! stays, because the history import merges two datasets through it.
//!
//! Every field is regridded onto the common 0.25 degree grid
//! ([`crate::regrid`]): a product published at 0.125 degree has four cells
//! meeting at each of that grid's nodes, exactly as GlobCurrent's do.

use std::ops::Range;

use zarrs::array::ArraySubset;

use crate::error::{Result, ZarrError};
use crate::parallel::try_join;
#[cfg(test)]
use crate::regrid::to_era5_grid;
use crate::regrid::{CellGrid, native_window, place, to_era5_window};
use crate::source::{Field, FieldSource, Step, Variable, Window, step_at_hour, steps_between};
use crate::store::{ReadArray, open_array, open_http, read_axis_f32, read_err, read_time_axis};
use crate::time::Utc;

/// What distinguishes one product's store from another's.
#[derive(Debug, Clone, Copy)]
pub struct ArcoSpec {
    /// Short name, for logs and messages.
    pub name: &'static str,
    /// What the two arrays are the components of.
    pub variable: Variable,
    /// The eastward component's array.
    pub u_path: &'static str,
    /// The northward component's array.
    pub v_path: &'static str,
    /// A depth axis and the value wanted on it, for a store that has one:
    /// GlobCurrent keeps the surface and 15 m down in one array.
    pub level: Option<(&'static str, f32)>,
    /// A per-cell measurement time beside the components, for a store that
    /// has one: a swath product records when each cell was seen, which is
    /// what decides between two passes over the same cell.
    pub time_path: Option<&'static str>,
    /// A daily product: each time is the field for its day, and is filed
    /// under the day's midnight whatever hour the publisher stamps it with,
    /// so the midnights an import asks for are found.
    pub daily: bool,
}

/// How a store packs its values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Packed {
    I16,
    I32,
}

/// One step's components on the store's own grid, and the per-cell
/// measurement time if the store has one.
pub type Timed = (Vec<f32>, Vec<f32>, Option<Vec<f64>>);

/// An open handle on one time-chunked store.
pub struct ArcoStore {
    spec: ArcoSpec,
    u: ReadArray,
    v: ReadArray,
    /// The per-cell time array, its fill value and its epoch in Unix
    /// seconds, for a store that has one.
    timed: Option<(ReadArray, f64, f64)>,
    /// Index along the depth axis, for a store that has one.
    level: Option<u64>,
    grid: CellGrid,
    packed: Packed,
    scale: f32,
    offset: f32,
    fill: i64,
    /// Hours since the Unix epoch, sorted.
    times: Vec<i64>,
    /// The packed values are kelvin, and leave here in Celsius.
    kelvin: bool,
    /// What every read asks the server for (M102): the whole grid unless a
    /// regional fetch said otherwise.
    window: Window,
}

impl std::fmt::Debug for ArcoStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ArcoStore")
            .field("name", &self.spec.name)
            .field("steps", &self.times.len())
            .field(
                "coverage",
                &self.coverage().map(|(a, b)| (a.to_iso(), b.to_iso())),
            )
            .finish()
    }
}

/// A per-cell time in seconds since the Unix epoch, or NaN where the store
/// has none. `base_s` is the store's own epoch in Unix seconds.
pub fn unpack_time(raw: f64, fill: f64, base_s: f64) -> f64 {
    if raw == fill || !raw.is_finite() {
        f64::NAN
    } else {
        base_s + raw
    }
}

/// A daily product's times, each moved to its day's midnight.
///
/// Refused if two times fall on one day: that is not a daily product, and
/// filing both under one midnight would lose one of them.
pub fn daily_hours(hours: &[i64]) -> Result<Vec<i64>> {
    let days: Vec<i64> = hours.iter().map(|h| h.div_euclid(24) * 24).collect();
    if days.windows(2).any(|w| w[1] <= w[0]) {
        return Err(ZarrError::Layout(
            "a daily product holds two times on one day".to_owned(),
        ));
    }
    Ok(days)
}

/// One packed value in physical units, or NaN where the store has none.
pub fn unpack(raw: i64, fill: i64, scale: f32, offset: f32) -> f32 {
    if raw == fill {
        f32::NAN
    } else {
        raw as f32 * scale + offset
    }
}

/// The grid two coordinate axes describe.
///
/// Read off the store rather than assumed, because the products are not all
/// on one grid; checked, because the regridder takes a regular grid that
/// spans the circle and would quietly put a field in the wrong place on
/// anything else.
pub fn cell_grid_of(lat: &[f32], lon: &[f32]) -> Result<CellGrid> {
    let spacing = |axis: &[f32], what: &str| -> Result<f64> {
        let (Some(&first), Some(&last)) = (axis.first(), axis.last()) else {
            return Err(ZarrError::Layout(format!("the {what} axis is empty")));
        };
        if axis.len() < 2 {
            return Err(ZarrError::Layout(format!(
                "the {what} axis has one entry, which is not a grid"
            )));
        }
        let step = (f64::from(last) - f64::from(first)) / (axis.len() - 1) as f64;
        // A hundredth of a cell: far more than `f32` rounding on a coordinate,
        // far less than a row out of place.
        let even = axis.iter().enumerate().all(|(i, &value)| {
            (f64::from(value) - (f64::from(first) + step * i as f64)).abs() <= step.abs() * 0.01
        });
        if step == 0.0 || !even {
            return Err(ZarrError::Layout(format!(
                "the {what} axis is not evenly spaced"
            )));
        }
        Ok(step)
    };
    let dlat = spacing(lat, "latitude")?;
    let dlon = spacing(lon, "longitude")?;
    if dlon <= 0.0 || (dlon * lon.len() as f64 - 360.0).abs() > dlon * 0.01 {
        return Err(ZarrError::Layout(format!(
            "the longitude axis spans {} degrees, not the whole circle",
            dlon * lon.len() as f64
        )));
    }
    Ok(CellGrid {
        lat0: f64::from(lat[0]),
        dlat,
        nlat: lat.len(),
        lon0: f64::from(lon[0]),
        // The spacing a grid that spans the circle *is*, rather than the one
        // its `f32` axis spells: a fifth of a degree read off two `f32` ends
        // is not exactly 0.2, and the regridder holds the span to a
        // millionth of a degree (OSTIA's 0.2 degree copy, M93).
        dlon: 360.0 / lon.len() as f64,
        nlon: lon.len(),
    })
}

/// The subsets of one time step a window reads: this time, this level if
/// the store has one, and the rows and columns [`native_window`] names —
/// one block, or two where the window crosses the store's first column.
///
/// The global window is one subset of every row and column, which is what
/// every read asked for before there were regional projects.
pub fn subset_for(
    window: &Window,
    grid: CellGrid,
    index: u64,
    level: Option<u64>,
) -> Vec<ArraySubset> {
    let native = native_window(grid, window);
    let rows = native.rows.start as u64..native.rows.end as u64;
    native
        .columns
        .iter()
        .map(|columns| {
            let mut ranges: Vec<Range<u64>> = Vec::with_capacity(4);
            ranges.push(index..index + 1);
            if let Some(level) = level {
                ranges.push(level..level + 1);
            }
            ranges.push(rows.clone());
            ranges.push(columns.start as u64..columns.end as u64);
            ArraySubset::new_with_ranges(&ranges)
        })
        .collect()
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

impl ArcoStore {
    /// Opens a store and validates its layout against `spec`.
    pub fn open(url: &str, spec: ArcoSpec) -> Result<Self> {
        let name = spec.name;
        let store = open_http(url)?;
        let (u, v) = try_join(
            || open_array(&store, spec.u_path),
            || open_array(&store, spec.v_path),
        )?;
        if u.shape() != v.shape() {
            return Err(ZarrError::Layout(format!(
                "{name} u and v have different shapes: {:?} vs {:?}",
                u.shape(),
                v.shape()
            )));
        }
        let dimensions = if spec.level.is_some() { 4 } else { 3 };
        if u.shape().len() != dimensions {
            return Err(ZarrError::Layout(format!(
                "{name} {} has shape {:?}, expected {dimensions} dimensions",
                spec.u_path,
                u.shape()
            )));
        }

        let dtype = format!("{:?}", u.data_type()).to_ascii_lowercase();
        let packed = if dtype.contains("int16") {
            Packed::I16
        } else if dtype.contains("int32") {
            Packed::I32
        } else {
            return Err(ZarrError::Layout(format!(
                "{name} is stored as {dtype}; this reader takes scaled int16 or int32"
            )));
        };
        if format!("{:?}", v.data_type()).to_ascii_lowercase() != dtype {
            return Err(ZarrError::Layout(format!(
                "{name} u and v are stored as different types"
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
                "{name} u and v are scaled differently"
            )));
        }
        let fill = match (packed, u.fill_value().as_ne_bytes()) {
            (Packed::I16, [a, b]) => i64::from(i16::from_ne_bytes([*a, *b])),
            (Packed::I32, [a, b, c, d]) => i64::from(i32::from_ne_bytes([*a, *b, *c, *d])),
            (_, other) => {
                return Err(ZarrError::Layout(format!(
                    "{name} fill value is {} bytes, which does not match its type",
                    other.len()
                )));
            }
        };

        let kelvin = u
            .attributes()
            .get("units")
            .and_then(|v| v.as_str())
            .is_some_and(|units| units.eq_ignore_ascii_case("kelvin") || units == "K");
        let steps = u.shape()[0];
        let ((lat, lon), times) = try_join(
            || {
                try_join(
                    || read_axis_f32(&store, "/latitude", "the latitude coordinate"),
                    || read_axis_f32(&store, "/longitude", "the longitude coordinate"),
                )
            },
            || read_time_axis(&store, "/time", steps),
        )?;
        let grid = cell_grid_of(&lat, &lon)?;
        let rows_and_columns = &u.shape()[dimensions - 2..];
        if rows_and_columns != [grid.nlat as u64, grid.nlon as u64] {
            return Err(ZarrError::Layout(format!(
                "{name} arrays are {rows_and_columns:?} but its axes are {} x {}",
                grid.nlat, grid.nlon
            )));
        }

        let level = match spec.level {
            None => None,
            Some((path, wanted)) => {
                let axis = read_axis_f32(&store, path, "the depth coordinate")?;
                if u.shape()[1] != axis.len() as u64 {
                    return Err(ZarrError::Layout(format!(
                        "{name} depth axis has {} entries but the arrays have {}",
                        axis.len(),
                        u.shape()[1]
                    )));
                }
                let at = axis.iter().position(|&e| e == wanted).ok_or_else(|| {
                    ZarrError::Layout(format!(
                        "{name} has no level at {wanted}; its depth axis is {axis:?}"
                    ))
                })?;
                Some(at as u64)
            }
        };

        let timed = match spec.time_path {
            None => None,
            Some(path) => {
                let array = open_array(&store, path)?;
                if array.shape() != u.shape() {
                    return Err(ZarrError::Layout(format!(
                        "{name} {path} has shape {:?}, not the components' {:?}",
                        array.shape(),
                        u.shape()
                    )));
                }
                let units = array
                    .attributes()
                    .get("units")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| ZarrError::Layout(format!("{name} {path} has no units")))?
                    .to_owned();
                let (epoch, unit) = crate::store::parse_time_units(&units)?;
                if unit != crate::store::TimeUnit::Seconds {
                    return Err(ZarrError::Layout(format!(
                        "{name} {path} counts in {unit:?}; a measurement time is in seconds"
                    )));
                }
                let base_s = epoch.hours_since_unix_epoch() as f64 * 3600.0;
                let fill = match array.fill_value().as_ne_bytes() {
                    [a, b, c, d, e, f, g, h] => {
                        f64::from_ne_bytes([*a, *b, *c, *d, *e, *f, *g, *h])
                    }
                    other => {
                        return Err(ZarrError::Layout(format!(
                            "{name} {path} fill value is {} bytes, expected a float64",
                            other.len()
                        )));
                    }
                };
                Some((array, fill, base_s))
            }
        };

        let times = if spec.daily {
            daily_hours(&times)?
        } else {
            times
        };

        Ok(Self {
            spec,
            kelvin,
            u,
            v,
            timed,
            level,
            grid,
            packed,
            scale,
            offset,
            fill,
            times,
            window: Window::global(),
        })
    }

    /// The grid the store publishes on.
    pub fn native_grid(&self) -> CellGrid {
        self.grid
    }

    /// One step of an array, on the store's whole grid: the window's
    /// subsets read through `read`, and every cell outside them `missing`.
    fn gather<T: Copy>(
        &self,
        index: u64,
        what: &str,
        missing: T,
        read: impl Fn(&ArraySubset) -> Result<Vec<T>>,
    ) -> Result<Vec<T>> {
        let subsets = subset_for(&self.window, self.grid, index, self.level);
        let mut blocks = Vec::with_capacity(subsets.len());
        for subset in &subsets {
            let block = read(subset)?;
            if block.len() != subset.num_elements_usize() {
                return Err(ZarrError::Layout(format!(
                    "{what} at step {index} holds {} values, expected {}",
                    block.len(),
                    subset.num_elements_usize()
                )));
            }
            blocks.push(block);
        }
        if let [whole] = subsets.as_slice()
            && whole.num_elements_usize() == self.grid.len()
        {
            return Ok(blocks.pop().unwrap_or_default());
        }
        let mut out = vec![missing; self.grid.len()];
        for (subset, block) in subsets.iter().zip(&blocks) {
            let ranges = subset.to_ranges();
            let [.., rows, columns] = ranges.as_slice() else {
                continue;
            };
            place(
                self.grid,
                &mut out,
                rows.start as usize..rows.end as usize,
                columns.start as usize..columns.end as usize,
                block,
            );
        }
        Ok(out)
    }

    /// The per-cell measurement time at one step, in seconds since the Unix
    /// epoch with NaN where no measurement was made; `None` for a store
    /// without one.
    fn read_times(&self, index: u64) -> Result<Option<Vec<f64>>> {
        let Some((array, fill, base_s)) = &self.timed else {
            return Ok(None);
        };
        let what = format!("the {} measurement time", self.spec.name);
        let raw = self.gather(index, &what, *fill, |subset| {
            array
                .retrieve_array_subset::<Vec<f64>>(subset)
                .map_err(read_err(&what))
        })?;
        Ok(Some(
            raw.into_iter()
                .map(|r| unpack_time(r, *fill, *base_s))
                .collect(),
        ))
    }

    /// The components and the measurement time at one step, on the store's
    /// own grid: what a merge of several stores works from.
    pub fn read_native_timed(&self, step: &Step) -> Result<Timed> {
        let ((u, v), times) = try_join(
            || {
                try_join(
                    || self.read_native(&self.u, step.index, "u"),
                    || self.read_native(&self.v, step.index, "v"),
                )
            },
            || self.read_times(step.index),
        )?;
        Ok((u, v, times))
    }

    /// Hours since the Unix epoch of every step, in order.
    pub fn hours(&self) -> &[i64] {
        &self.times
    }

    /// One component at one step, on the store's own grid, in physical units
    /// with NaN where masked.
    fn read_native(&self, array: &ReadArray, index: u64, name: &str) -> Result<Vec<f32>> {
        let what = format!("the {} {name} field", self.spec.name);
        let raw: Vec<i64> = self.gather(index, &what, self.fill, |subset| {
            Ok(match self.packed {
                Packed::I16 => array
                    .retrieve_array_subset::<Vec<i16>>(subset)
                    .map_err(read_err(&what))?
                    .into_iter()
                    .map(i64::from)
                    .collect(),
                Packed::I32 => array
                    .retrieve_array_subset::<Vec<i32>>(subset)
                    .map_err(read_err(&what))?
                    .into_iter()
                    .map(i64::from)
                    .collect(),
            })
        })?;
        Ok(raw
            .into_iter()
            .map(|r| unpack(r, self.fill, self.scale, self.offset))
            .collect())
    }
}

impl FieldSource for ArcoStore {
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
        if self.spec.variable.is_scalar() {
            let mut values = self.read_native(&self.u, step.index, "value")?;
            if self.kelvin {
                for value in &mut values {
                    *value -= 273.15;
                }
            }
            return Ok(vec![Field {
                variable: self.spec.variable,
                u: to_era5_window(self.grid, &values, &self.window),
                v: Vec::new(),
            }]);
        }
        let (u, v) = try_join(
            || self.read_native(&self.u, step.index, "u"),
            || self.read_native(&self.v, step.index, "v"),
        )?;
        let u = to_era5_window(self.grid, &u, &self.window);
        let v = to_era5_window(self.grid, &v, &self.window);
        // A field with nothing in it is a time the store lists but has not
        // written yet: just after midnight the time axis already names the
        // new day. That is an empty field, not a failure — the newest steps
        // of a near-real-time product are usually empty, and one of them
        // must not cost the import the whole product.
        Ok(vec![Field {
            variable: self.spec.variable,
            u,
            v,
        }])
    }

    /// The Marine Data Store answers a subset with only the chunks it
    /// touches, so a regional fetch asks for its window (M102).
    fn set_window(&mut self, window: Window) {
        self.window = window;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn axis(first: f32, step: f32, count: usize) -> Vec<f32> {
        (0..count).map(|i| first + step * i as f32).collect()
    }

    /// A regional read asks the store for its window's cells and no others:
    /// the rows and columns round 40 N..60 N, 20 W..20 E on a grid that
    /// starts at -179.875 and runs north, at the surface level; and two
    /// blocks for a window across the store's seam at 180 degrees.
    #[test]
    fn arco_reads_only_the_window() {
        let grid = CellGrid::GLOBCURRENT;
        let w = Window {
            i0: 1360,
            ni: 161,
            j0: 120,
            nj: 81,
        };
        // 60 N is row (60 + 89.875) / 0.25 = 599.5, 40 N is 519.5: rows 519
        // to 600 bracket them, and a row more each side is 518..602. 340 E is
        // column 639.5 from -179.875, 20 E is 799.5: 638..802 likewise.
        let subsets = subset_for(&w, grid, 7, Some(0));
        assert_eq!(
            subsets,
            [ArraySubset::new_with_ranges(&[
                7..8,
                0..1,
                518..602,
                638..802
            ])]
        );

        let across = Window {
            i0: 640,
            ni: 161,
            j0: 320,
            nj: 81,
        };
        let subsets = subset_for(&across, grid, 7, None);
        assert_eq!(
            subsets,
            [
                // 160 E is column 1359.5, so from 1358 to the end of the row,
                ArraySubset::new_with_ranges(&[7..8, 318..402, 1358..1440]),
                // and 200 E (-160) is column 79.5, so from 0 to 81.
                ArraySubset::new_with_ranges(&[7..8, 318..402, 0..82]),
            ]
        );
        let cells: u64 = subsets.iter().map(ArraySubset::num_elements).sum();
        assert!(
            cells * 50 < grid.len() as u64,
            "{cells} cells of {}",
            grid.len()
        );

        // The whole grid is one subset of everything, as it always was.
        assert_eq!(
            subset_for(&Window::global(), grid, 7, None),
            [ArraySubset::new_with_ranges(&[7..8, 0..720, 0..1440])]
        );
    }

    /// The two grids these stores are published on, read off their axes.
    #[test]
    fn a_grid_is_read_off_its_axes() {
        let eighth = cell_grid_of(&axis(-89.9375, 0.125, 1440), &axis(-179.9375, 0.125, 2880))
            .expect("the 0.125 degree grid");
        assert_eq!(
            eighth,
            CellGrid {
                lat0: -89.9375,
                dlat: 0.125,
                nlat: 1440,
                lon0: -179.9375,
                dlon: 0.125,
                nlon: 2880,
            }
        );
        let quarter = cell_grid_of(&axis(-89.875, 0.25, 720), &axis(-179.875, 0.25, 1440))
            .expect("the 0.25 degree grid");
        assert_eq!(quarter, CellGrid::GLOBCURRENT);
    }

    /// A fifth of a degree as `f32` endpoints spans the circle only to
    /// within rounding; the grid made of it spans it exactly, so the
    /// regridder takes it.
    #[test]
    fn a_spacing_f32_cannot_spell_is_snapped_to_the_circle() {
        let lon: Vec<f32> = (0..1800).map(|i| -179.9 + 0.2 * i as f32).collect();
        let grid = cell_grid_of(&axis(-89.9, 0.2, 900), &lon).expect("a grid");
        assert_eq!(grid.dlon * grid.nlon as f64, 360.0);
        let regridded = to_era5_grid(grid, &vec![1.0; grid.len()]);
        assert!(regridded.iter().all(|v| *v == 1.0));
    }

    /// An axis that is not evenly spaced, or does not go all the way round,
    /// is not a grid the regridder can be handed.
    #[test]
    fn an_irregular_or_partial_grid_is_refused() {
        let mut lat = axis(-89.875, 0.25, 720);
        for value in lat.iter_mut().skip(360) {
            *value += 0.25;
        }
        assert!(cell_grid_of(&lat, &axis(-179.875, 0.25, 1440)).is_err());
        // Half the circle.
        assert!(cell_grid_of(&axis(-89.875, 0.25, 720), &axis(-179.875, 0.25, 720)).is_err());
        assert!(cell_grid_of(&[0.0], &axis(-179.875, 0.25, 1440)).is_err());
    }

    /// 2026-10-01T00Z is 20 727 days after 1970 (see `store.rs`'s working)
    /// and 13 422 days after 1990; the fill is NaN, and so is anything that
    /// is not a number.
    #[test]
    fn a_measurement_time_is_placed_on_the_unix_epoch_and_a_fill_is_missing() {
        let since_1990 = 7_305.0 * 86_400.0;
        let fill = -2_147_483_647.0;
        assert!(unpack_time(fill, fill, since_1990).is_nan());
        assert!(unpack_time(f64::NAN, fill, since_1990).is_nan());
        let at = unpack_time(13_422.0 * 86_400.0, fill, since_1990);
        assert_eq!(at, 20_727.0 * 86_400.0);
        assert_eq!(at, 1_790_812_800.0);
    }

    /// OISST and Geo-Polar stamp a day at noon; filed at midnight, a day is
    /// found where an import looks for it. Two stamps on one day are refused.
    #[test]
    fn a_daily_product_is_filed_under_its_midnights() {
        assert_eq!(daily_hours(&[12, 36, 60]).unwrap(), vec![0, 24, 48]);
        assert_eq!(daily_hours(&[-12]).unwrap(), vec![-24]);
        assert!(daily_hours(&[0, 12]).is_err());
    }

    #[test]
    fn a_packed_value_is_scaled_and_a_fill_is_missing() {
        assert!(unpack(-32767, -32767, 0.01, 0.0).is_nan());
        assert!((unpack(1234, -32767, 0.01, 0.0) - 12.34).abs() < 1e-6);
        assert!((unpack(-5000, -2_147_483_647, 1e-4, 0.0) + 0.5).abs() < 1e-7);
        assert!((unpack(10, 0, 2.0, 1.0) - 21.0).abs() < 1e-6);
    }

    /// The regrid, by the property that defines it rather than by its own
    /// formula: a field that *is* its longitude comes out as the longitude at
    /// every node, and a constant field comes out constant. Checked on the
    /// 0.125 degree grid, which is the one new to this reader.
    #[test]
    fn a_finer_grid_regrids_to_the_place_each_node_is() {
        let grid =
            cell_grid_of(&axis(-89.9375, 0.125, 1440), &axis(-179.9375, 0.125, 2880)).unwrap();
        let lon_field: Vec<f32> = (0..grid.nlat)
            .flat_map(|_| (0..grid.nlon).map(|i| -179.9375 + 0.125 * i as f32))
            .collect();
        let out = to_era5_grid(grid, &lon_field);
        let node = |lat: f64, lon: f64| {
            let j = ((90.0 - lat) / 0.25).round() as usize;
            let i = (lon / 0.25).round() as usize;
            out[j * crate::source::NI as usize + i]
        };
        assert!((node(0.0, 10.0) - 10.0).abs() < 1e-3, "{}", node(0.0, 10.0));
        assert!(
            (node(45.0, 350.0) + 10.0).abs() < 1e-3,
            "{}",
            node(45.0, 350.0)
        );
        assert!((node(-60.0, 179.75) - 179.75).abs() < 1e-3);
        assert!((node(-60.0, 180.25) + 179.75).abs() < 1e-3);

        let constant = to_era5_grid(grid, &vec![3.5; grid.len()]);
        assert!(constant.iter().all(|v| (v - 3.5).abs() < 1e-6));
    }
}

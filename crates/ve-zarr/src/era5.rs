//! 10 m wind out of the ARCO-ERA5 Zarr store.
//!
//! The store is public and anonymous, so a plain HTTP reader is enough; no
//! Google Cloud credentials are involved.
//!
//! # Why the time axis is not the coverage
//!
//! ARCO-ERA5 is rewritten as ECMWF publishes hours, and its time axis is
//! preallocated far past its data: the arrays are shaped out to 2050 and the
//! chunks for hours nobody has produced yet are simply absent. The axis is
//! therefore no evidence that a step exists. The root attributes are what say
//! how far the data runs:
//!
//! - `valid_time_start`, the first day written;
//! - `valid_time_stop`, the last day of the final, quality-controlled stream,
//!   which trails real time by two to three months;
//! - `valid_time_stop_era5t`, the last day of ERA5T, the preliminary stream
//!   that carries the record on to about five days ago.
//!
//! [`Era5Store::open`] clamps the axis to those days, so [`Era5Store`]'s
//! coverage describes hours that exist rather than the shape of an array, and
//! it keeps the boundary between the two streams: ERA5T hours export like any
//! other, but ECMWF may revise them, so the UI says where they start.
//!
//! A store publishing none of those attributes -- the frozen WeatherBench
//! snapshot this app read before, whose root attributes are an empty object --
//! is taken at its axis's word and reports no boundary.
//!
//! # Why the arrays need no reprojection
//!
//! The arrays are `(time, latitude, longitude)` with latitude running from
//! +90 down to -90 and longitude from 0 up to 359.75. GRIB2 scanning mode 0
//! is north to south from `La1` and west to east from `Lo1 = 0`. Those are the
//! same order, so a decoded chunk goes into section 7 as it is.
//! [`Era5Store::open`] checks the coordinates rather than trusting that,
//! because a silent change upstream would produce a file that looks valid and
//! is upside down.

use zarrs::config::MetadataRetrieveVersion;
use zarrs::group::Group;
use zarrs::storage::ReadableStorage;

use crate::error::{Result, ZarrError};
use crate::parallel::try_join;
use crate::source::{
    Field, FieldSource, NI, NJ, POINTS_PER_STEP, Step, Variable, step_at_hour, steps_between,
};
use crate::store::{
    ReadArray, check_axis, open_array, open_http, read_axis_f32, read_err, read_time_axis,
};
use crate::time::Utc;

/// The default store: ERA5 at 0.25 degrees, hourly, one chunk per time step,
/// updated as ECMWF publishes rather than frozen at a snapshot.
pub const DEFAULT_STORE_URL: &str = "https://storage.googleapis.com/gcp-public-data-arco-era5/ar/full_37-1h-0p25deg-chunk-1.zarr-v3";

const U_PATH: &str = "/10m_u_component_of_wind";
const V_PATH: &str = "/10m_v_component_of_wind";

/// How far the store's data runs, as its root attributes describe it.
///
/// Every field is optional: a store that publishes none of them is a fixed
/// snapshot, and its axis is its coverage.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct Extent {
    /// First hour written.
    first: Option<Utc>,
    /// Last hour of the final stream.
    final_last: Option<Utc>,
    /// Last hour written, ERA5T included.
    last: Option<Utc>,
}

impl Extent {
    /// Reads the `valid_time_*` attributes off the root group.
    fn read(store: &ReadableStorage) -> Result<Self> {
        // Explicitly V2, for the reason [`crate::store::open_array`] gives.
        let group = Group::open_opt(store.clone(), "/", &MetadataRetrieveVersion::V2)
            .map_err(|e| ZarrError::Open(format!("the root group: {e}")))?;
        let attrs = group.attributes();

        // The attributes are dates, not instants. A start opens its day and a
        // stop closes it: the last hour written on `valid_time_stop` is 23:00.
        let day = |key: &str, hour: u8| -> Result<Option<Utc>> {
            let Some(value) = attrs.get(key) else {
                return Ok(None);
            };
            let text = value.as_str().ok_or_else(|| {
                ZarrError::Layout(format!("the {key} attribute is not a string: {value}"))
            })?;
            Utc::parse(&format!("{}T{hour:02}:00", text.trim()))
                .ok_or_else(|| {
                    ZarrError::Layout(format!("the {key} attribute {text:?} is not a date"))
                })
                .map(Some)
        };

        let final_last = day("valid_time_stop", 23)?;
        let era5t_last = day("valid_time_stop_era5t", 23)?;
        Ok(Self {
            first: day("valid_time_start", 0)?,
            final_last,
            last: era5t_last.or(final_last),
        })
    }

    /// The first preliminary hour, if the final stream stops short of the
    /// last one written.
    fn provisional_from(&self) -> Option<Utc> {
        let (final_last, last) = (self.final_last?, self.last?);
        (final_last < last)
            .then(|| Utc::from_hours_since_unix_epoch(final_last.hours_since_unix_epoch() + 1))
    }
}

/// An open handle on the ERA5 store.
pub struct Era5Store {
    u: ReadArray,
    v: ReadArray,
    /// Valid time of every written step, as hours since the Unix epoch.
    /// Sorted, and clamped to what the store says it holds.
    times: Vec<i64>,
    /// Position in the arrays of `times[0]`. The axis starts before the data
    /// does, so a step's place in `times` is not its chunk index.
    base: u64,
    /// First hour that comes from ERA5T rather than the final stream.
    provisional_from: Option<Utc>,
}

impl std::fmt::Debug for Era5Store {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Era5Store")
            .field("steps", &self.times.len())
            .field(
                "coverage",
                &self.coverage().map(|(a, b)| (a.to_iso(), b.to_iso())),
            )
            .field(
                "provisional_from",
                &self.provisional_from.map(|t| t.to_iso()),
            )
            .finish()
    }
}

impl Era5Store {
    /// Opens the store and validates its layout.
    pub fn open(url: &str) -> Result<Self> {
        let store = open_http(url)?;
        let (u, v) = try_join(|| open_array(&store, U_PATH), || open_array(&store, V_PATH))?;

        for (path, array) in [(U_PATH, &u), (V_PATH, &v)] {
            match array.shape() {
                [_, nj, ni] if *nj == NJ && *ni == NI => {}
                other => {
                    return Err(ZarrError::Layout(format!(
                        "{path} has shape {other:?}, expected [time, {NJ}, {NI}]"
                    )));
                }
            }
        }
        if u.shape()[0] != v.shape()[0] {
            return Err(ZarrError::Layout(format!(
                "u has {} time steps but v has {}",
                u.shape()[0],
                v.shape()[0]
            )));
        }

        let ((lat, lon), (axis, extent)) = try_join(
            || {
                try_join(
                    || read_axis_f32(&store, "/latitude", "the latitude coordinate"),
                    || read_axis_f32(&store, "/longitude", "the longitude coordinate"),
                )
            },
            || {
                try_join(
                    || read_time_axis(&store, "/time", u.shape()[0]),
                    || Extent::read(&store),
                )
            },
        )?;
        check_axis("latitude", &lat, NJ as usize, 90.0, -0.25)?;
        check_axis("longitude", &lon, NI as usize, 0.0, 0.25)?;

        let (base, times) = clamp_axis(&axis, &extent)?;
        let provisional_from = extent.provisional_from().filter(|t| {
            times
                .last()
                .is_some_and(|&h| t.hours_since_unix_epoch() <= h)
        });

        Ok(Self {
            u,
            v,
            times,
            base,
            provisional_from,
        })
    }

    /// How many written time steps the store holds.
    pub fn step_count(&self) -> usize {
        self.times.len()
    }

    fn read_component(&self, array: &ReadArray, step: &Step, name: &str) -> Result<Vec<f32>> {
        let index = self.base + step.index;
        let field = array
            .retrieve_chunk::<Vec<f32>>(&[index, 0, 0])
            .map_err(read_err(&format!("the 10 m {name} wind chunk")))?;
        if field.len() != POINTS_PER_STEP {
            return Err(ZarrError::Layout(format!(
                "the {name} chunk at {} holds {} values, expected {POINTS_PER_STEP}",
                step.valid_time.to_iso(),
                field.len()
            )));
        }
        // ERA5 wind has no missing points. A NaN here means an unwritten
        // chunk -- the store's attributes reaching past the hours actually
        // published -- and must not quietly become a masked message.
        if let Some(i) = field.iter().position(|v| !v.is_finite()) {
            return Err(ZarrError::Layout(format!(
                "the {name} chunk at index {index} ({}) is not fully written: non-finite value at {i}",
                step.valid_time.to_iso()
            )));
        }
        Ok(field)
    }
}

/// Cuts a time axis down to the hours an [`Extent`] claims are written, and
/// returns where the survivors start in the arrays.
fn clamp_axis(axis: &[i64], extent: &Extent) -> Result<(u64, Vec<i64>)> {
    let lo = match extent.first {
        Some(t) => axis.partition_point(|&h| h < t.hours_since_unix_epoch()),
        None => 0,
    };
    let hi = match extent.last {
        Some(t) => axis.partition_point(|&h| h <= t.hours_since_unix_epoch()),
        None => axis.len(),
    };
    if lo >= hi {
        let described = |t: Option<Utc>| t.map_or("unset".to_string(), |t| t.to_iso());
        return Err(ZarrError::Layout(format!(
            "the store says it holds {} to {}, which leaves no hour of its {} step axis",
            described(extent.first),
            described(extent.last),
            axis.len()
        )));
    }
    Ok((lo as u64, axis[lo..hi].to_vec()))
}

impl FieldSource for Era5Store {
    fn name(&self) -> &'static str {
        "ERA5"
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

    fn provisional_from(&self) -> Option<Utc> {
        self.provisional_from
    }

    fn step_at(&self, time: Utc) -> Option<Step> {
        step_at_hour(&self.times, time)
    }

    fn steps_in_range(&self, start: Utc, end: Utc) -> Result<Vec<Step>> {
        steps_between(&self.times, start, end, "ERA5")
    }

    fn read_step(&self, step: &Step) -> Result<Vec<Field>> {
        Ok(vec![Field {
            variable: Variable::Wind10m,
            u: self.read_component(&self.u, step, "u")?,
            v: self.read_component(&self.v, step, "v")?,
        }])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn component(path: &str, value: Option<f32>) -> ReadArray {
        use std::sync::Arc;
        use zarrs::array::{Array, ArrayBuilder, data_type};
        use zarrs::storage::store::MemoryStore;

        let store = Arc::new(MemoryStore::new());
        let array = ArrayBuilder::new(
            vec![2, NJ, NI],
            vec![1, NJ, NI],
            data_type::float32(),
            f32::NAN,
        )
        .build(store.clone(), path)
        .expect("fixture array");
        array.store_metadata().expect("metadata");
        if let Some(value) = value {
            array
                .store_chunk(&[1, 0, 0], vec![value; (NI * NJ) as usize])
                .expect("fixture chunk");
        }
        let readable: ReadableStorage = store;
        Array::open(readable, path).expect("readable fixture")
    }

    #[test]
    fn component_reads_keep_their_values_and_absolute_hour() {
        let time = utc(2024, 1, 1, 0);
        let source = Era5Store {
            u: component(U_PATH, Some(-3.0)),
            v: component(V_PATH, Some(4.0)),
            times: vec![time.hours_since_unix_epoch()],
            base: 1,
            provisional_from: None,
        };
        let step = source.step_at(time).expect("hour exists");
        let fields = source.read_step(&step).expect("complete vector");
        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0].variable, Variable::Wind10m);
        assert!(fields[0].u.iter().all(|&v| v == -3.0));
        assert!(fields[0].v.iter().all(|&v| v == 4.0));

        // A successful u read cannot hide a missing v chunk.
        let source = Era5Store {
            v: component(V_PATH, None),
            ..source
        };
        assert!(
            source
                .read_step(&step)
                .expect_err("incomplete vector")
                .to_string()
                .contains("not fully written")
        );
    }

    fn utc(year: i32, month: u8, day: u8, hour: u8) -> Utc {
        Utc {
            year,
            month,
            day,
            hour,
        }
    }

    /// An hourly axis of `n` hours from `from`.
    fn axis(from: Utc, n: i64) -> Vec<i64> {
        let base = from.hours_since_unix_epoch();
        (0..n).map(|i| base + i).collect()
    }

    /// The axis of the live store starts in 1900 and runs to 2050; only the
    /// middle of it is written, and a step's place in the clamped list is
    /// that many chunks short of its index.
    #[test]
    fn an_axis_is_clamped_to_the_written_days() {
        let axis = axis(utc(2020, 1, 1, 0), 24 * 5);
        let extent = Extent {
            first: Some(utc(2020, 1, 2, 0)),
            final_last: Some(utc(2020, 1, 3, 23)),
            last: Some(utc(2020, 1, 4, 23)),
        };
        let (base, times) = clamp_axis(&axis, &extent).expect("clamps");
        assert_eq!(base, 24, "a day of unwritten hours precedes the data");
        assert_eq!(times.len(), 24 * 3);
        assert_eq!(
            Utc::from_hours_since_unix_epoch(times[0]),
            utc(2020, 1, 2, 0)
        );
        assert_eq!(
            Utc::from_hours_since_unix_epoch(times[times.len() - 1]),
            utc(2020, 1, 4, 23),
            "a stop day is written through its last hour"
        );
    }

    /// The frozen snapshot publishes no `valid_time_*` attributes at all.
    #[test]
    fn an_axis_without_attributes_is_left_alone() {
        let axis = axis(utc(2020, 1, 1, 0), 48);
        let (base, times) = clamp_axis(&axis, &Extent::default()).expect("clamps");
        assert_eq!(base, 0);
        assert_eq!(times, axis);
    }

    #[test]
    fn attributes_that_exclude_every_hour_are_refused() {
        let axis = axis(utc(2020, 1, 1, 0), 48);
        let extent = Extent {
            first: Some(utc(2030, 1, 1, 0)),
            final_last: None,
            last: Some(utc(2030, 1, 2, 23)),
        };
        let err = clamp_axis(&axis, &extent).expect_err("refuses");
        assert!(format!("{err}").contains("leaves no hour"), "{err}");
    }

    /// ERA5T begins the hour after the final stream ends.
    #[test]
    fn the_provisional_boundary_is_the_hour_after_the_final_stream() {
        let extent = Extent {
            first: Some(utc(1940, 1, 1, 0)),
            final_last: Some(utc(2026, 4, 30, 23)),
            last: Some(utc(2026, 8, 29, 23)),
        };
        assert_eq!(extent.provisional_from(), Some(utc(2026, 5, 1, 0)));
    }

    /// A store whose streams end together -- or which names only one -- has
    /// no preliminary hours to warn about.
    #[test]
    fn a_store_with_one_stream_reports_no_boundary() {
        assert_eq!(Extent::default().provisional_from(), None);
        let extent = Extent {
            first: Some(utc(1940, 1, 1, 0)),
            final_last: Some(utc(2026, 4, 30, 23)),
            last: Some(utc(2026, 4, 30, 23)),
        };
        assert_eq!(extent.provisional_from(), None);
    }
}

//! Local routing stores: `data(time, param, latitude, longitude)` as written
//! by routing_test and by our Zarr export. The coordinates are authoritative;
//! descriptive attributes and the order of the parameters are not assumed.

use std::ops::Range;
use std::path::Path;
use std::sync::Arc;

use zarrs::array::{Array, ArraySubset, data_type};
use zarrs::config::MetadataRetrieveVersion;
use zarrs::filesystem::FilesystemStore;
use zarrs::storage::ReadableStorage;

use crate::store::{ReadArray, parse_epoch, read_err};
use crate::{Result, ZarrError};

/// Metadata and an open local array. Data is read in bounded time slabs so
/// opening a month does not require a second, whole-store decoding buffer.
pub struct RoutingStore {
    data: ReadArray,
    /// UTC seconds, strictly increasing.
    pub times: Vec<i64>,
    /// Component names, in the order stored on disk.
    pub parameters: Vec<String>,
    /// Eastward longitude coordinates.
    pub longitude: Vec<f64>,
    /// North-to-south latitude coordinates.
    pub latitude: Vec<f64>,
}

impl std::fmt::Debug for RoutingStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RoutingStore")
            .field("shape", &self.data.shape())
            .field("parameters", &self.parameters)
            .finish_non_exhaustive()
    }
}

fn layout(message: impl Into<String>) -> ZarrError {
    ZarrError::Layout(message.into())
}

fn open_array(store: &ReadableStorage, name: &str, len: u64) -> Result<ReadArray> {
    let array = Array::open_opt(store.clone(), name, &MetadataRetrieveVersion::V3)
        .map_err(|e| ZarrError::Open(format!("{name}: {e}")))?;
    if array.shape() != [len] {
        return Err(layout(format!("{name} must have shape [{len}]")));
    }
    Ok(array)
}

fn numbers(array: &ReadArray, subset: &ArraySubset) -> Result<Vec<f64>> {
    macro_rules! read {
        ($ty:ty, $convert:expr) => {
            array
                .retrieve_array_subset::<Vec<$ty>>(subset)
                .map(|values| values.into_iter().map($convert).collect())
                .map_err(read_err("routing coordinates"))
        };
    }
    let dtype = array.data_type();
    if dtype.is::<data_type::Float64DataType>() {
        read!(f64, |v| v)
    } else if dtype.is::<data_type::Float32DataType>() {
        read!(f32, f64::from)
    } else if dtype.is::<data_type::Int64DataType>() {
        read!(i64, |v| v as f64)
    } else if dtype.is::<data_type::Int32DataType>() {
        read!(i32, f64::from)
    } else {
        Err(layout(format!("unsupported coordinate type {dtype:?}")))
    }
}

fn axis(store: &ReadableStorage, name: &str, len: u64, descending: bool) -> Result<Vec<f64>> {
    let array = open_array(store, name, len)?;
    let values = numbers(&array, &array.subset_all())?;
    // An f32 coordinate rounded near a pole can give the first interval a
    // few microdegrees of error. Spread that rounding across the whole axis
    // rather than accumulating it at every node (notably at 0.1 degrees).
    let delta = (values[values.len() - 1] - values[0]) / (values.len() - 1) as f64;
    if !delta.is_finite()
        || delta == 0.0
        || (delta < 0.0) != descending
        || values.iter().enumerate().any(|(i, value)| {
            !value.is_finite() || (*value - (values[0] + i as f64 * delta)).abs() > 1e-4
        })
    {
        return Err(layout(format!(
            "{name} must be regularly spaced and run {}",
            if descending {
                "north to south"
            } else {
                "west to east"
            }
        )));
    }
    if descending && values.iter().any(|v| !(-90.0..=90.0).contains(v)) {
        return Err(layout("latitude is outside -90 to 90 degrees"));
    }
    if !descending && delta * len as f64 > 360.0 + 1e-4 {
        return Err(layout("longitude spans more than one turn"));
    }
    Ok(values)
}

impl RoutingStore {
    /// Open a Zarr v3 directory, including one without a `.zarr` suffix.
    pub fn open(path: &Path) -> Result<Self> {
        let store: ReadableStorage = Arc::new(
            FilesystemStore::new(path)
                .map_err(|e| ZarrError::Open(format!("{}: {e}", path.display())))?,
        );
        let data = Array::open_opt(store.clone(), "/data", &MetadataRetrieveVersion::V3)
            .map_err(|e| ZarrError::Open(format!("/data: {e}")))?;
        let [nt, np, nj, ni] = data.shape() else {
            return Err(layout(
                "data must have dimensions (time, param, latitude, longitude)",
            ));
        };
        let (nt, np, nj, ni) = (*nt, *np, *nj, *ni);
        if nt == 0 || np == 0 || nj < 2 || ni < 2 || ni > u32::MAX as u64 || nj > u32::MAX as u64 {
            return Err(layout(
                "data needs times, parameters, and at least two rows and columns",
            ));
        }
        let dims = data.dimension_names().as_ref().map(|names| {
            names
                .iter()
                .map(|name| name.as_deref().unwrap_or(""))
                .collect::<Vec<_>>()
        });
        if dims.as_deref() != Some(&["time", "param", "latitude", "longitude"]) {
            return Err(layout(
                "data dimension_names must be time, param, latitude, longitude",
            ));
        }
        if !data.data_type().is::<data_type::Float16DataType>()
            && !data.data_type().is::<data_type::Float32DataType>()
        {
            return Err(layout("data must contain float16 or float32 components"));
        }
        if data.attributes().get("units").and_then(|v| v.as_str()) != Some("m s-1") {
            return Err(layout("data units must be m s-1"));
        }
        let param = open_array(&store, "/param", np)?;
        let parameters: Vec<String> = if param
            .data_type()
            .is::<data_type::FixedLengthUTF32DataType>()
        {
            param
                .retrieve_array_subset::<Vec<Vec<char>>>(&param.subset_all())
                .map_err(read_err("routing parameters"))?
                .into_iter()
                .map(|chars| chars.into_iter().collect())
                .collect()
        } else {
            param
                .retrieve_array_subset(&param.subset_all())
                .map_err(read_err("routing parameters"))?
        };
        for (i, name) in parameters.iter().enumerate() {
            if parameters[..i].contains(name) {
                return Err(layout(format!("duplicate parameter {name}")));
            }
        }
        let mut pairs = 0;
        for [u, v] in [["u10", "v10"], ["ucur", "vcur"]] {
            let has = |name| parameters.iter().any(|p| p == name);
            if has(u) != has(v) {
                return Err(layout(format!("parameters must contain both {u} and {v}")));
            }
            pairs += usize::from(has(u));
        }
        if pairs == 0 {
            return Err(layout(
                "parameters contain no u10/v10 wind or ucur/vcur current pair",
            ));
        }
        let time = open_array(&store, "/time", nt)?;
        if time
            .attributes()
            .get("calendar")
            .and_then(|v| v.as_str())
            .is_some_and(|calendar| {
                !["proleptic_gregorian", "gregorian", "standard"].contains(&calendar)
            })
        {
            return Err(layout("time must use a Gregorian calendar"));
        }
        let units = time
            .attributes()
            .get("units")
            .and_then(|v| v.as_str())
            .ok_or_else(|| layout("time has no units"))?;
        let base = parse_epoch(units)?.hours_since_unix_epoch();
        let times = numbers(&time, &time.subset_all())?
            .into_iter()
            .map(|h| {
                if !h.is_finite() || h.fract() != 0.0 || h.abs() > 100_000_000.0 {
                    return Err(layout(
                        "time offsets must be whole hours within the supported calendar",
                    ));
                }
                base.checked_add(h as i64)
                    .and_then(|h| h.checked_mul(3600))
                    .ok_or_else(|| layout("time is outside the supported calendar"))
            })
            .collect::<Result<Vec<_>>>()?;
        if times.windows(2).any(|w| w[0] >= w[1]) {
            return Err(layout("time must be strictly increasing"));
        }
        Ok(Self {
            longitude: axis(&store, "/longitude", ni, false)?,
            latitude: axis(&store, "/latitude", nj, true)?,
            times,
            parameters,
            data,
        })
    }

    /// Read a time slab in on-disk dimension order. NaNs remain missing;
    /// absent shards and inner chunks are decoded using the array's fill.
    pub fn read(&self, times: Range<usize>) -> Result<Vec<f32>> {
        if times.start >= times.end || times.end > self.times.len() {
            return Err(ZarrError::TimeRange(
                "routing time range is outside the store".into(),
            ));
        }
        let subset = ArraySubset::new_with_ranges(&[
            times.start as u64..times.end as u64,
            0..self.parameters.len() as u64,
            0..self.latitude.len() as u64,
            0..self.longitude.len() as u64,
        ]);
        if self.data.data_type().is::<data_type::Float16DataType>() {
            self.data
                .retrieve_array_subset::<Vec<half::f16>>(&subset)
                .map(|v| v.into_iter().map(half::f16::to_f32).collect())
                .map_err(read_err("routing fields"))
        } else {
            self.data
                .retrieve_array_subset(&subset)
                .map_err(read_err("routing fields"))
        }
    }
}

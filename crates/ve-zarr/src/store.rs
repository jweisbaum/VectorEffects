//! What the ERA5 and GlobCurrent readers share: opening an HTTP store, reading
//! a time axis, and checking that a coordinate axis is what the code assumes.

use std::sync::Arc;

use crate::http::HttpStore;
use zarrs::array::Array;
use zarrs::config::MetadataRetrieveVersion;
use zarrs::storage::{ReadableStorage, ReadableStorageTraits};

use crate::error::{Result, ZarrError};
use crate::time::Utc;

/// A Zarr array on a read-only store.
pub type ReadArray = Array<dyn ReadableStorageTraits>;

/// Opens an anonymous HTTP store.
pub fn open_http(url: &str) -> Result<ReadableStorage> {
    crate::codec::register();
    Ok(Arc::new(
        HttpStore::new(url).map_err(|e| ZarrError::Open(format!("{url}: {e}")))?,
    ))
}

/// Opens an array, insisting on Zarr V2.
///
/// The default open probes for a V3 `zarr.json` first. Google Cloud answers a
/// missing key with 404 and the probe moves on; the CloudFerro S3 behind
/// Copernicus Marine answers 403, which `zarrs` reports as an error. Both
/// stores this crate reads are V2, so the probe is skipped rather than
/// interpreted.
pub fn open_array(store: &ReadableStorage, path: &str) -> Result<ReadArray> {
    Array::open_opt(store.clone(), path, &MetadataRetrieveVersion::V2)
        .map_err(|e| ZarrError::Open(format!("{path}: {e}")))
}

/// Wraps a zarrs read error with what was being read.
pub fn read_err(what: &str) -> impl FnOnce(zarrs::array::ArrayError) -> ZarrError + '_ {
    move |source| ZarrError::Read {
        what: what.to_string(),
        source: Box::new(source),
    }
}

/// Reads a whole one-dimensional coordinate array as `f32`, whatever numeric
/// type it is stored as. GlobCurrent keeps its elevation axis as `int16`.
pub fn read_axis_f32(store: &ReadableStorage, path: &str, what: &str) -> Result<Vec<f32>> {
    let array = open_array(store, path)?;
    let [len] = array.shape() else {
        return Err(ZarrError::Layout(format!(
            "{path} has shape {:?}, expected one dimension",
            array.shape()
        )));
    };
    let len = *len;
    let dtype = format!("{:?}", array.data_type()).to_ascii_lowercase();
    macro_rules! read_as {
        ($t:ty) => {{
            #[allow(clippy::single_range_in_vec_init, reason = "one range per dimension")]
            let v = array
                .retrieve_array_subset::<Vec<$t>>(&[0..len])
                .map_err(read_err(what))?;
            Ok(v.into_iter().map(|x| x as f32).collect())
        }};
    }
    if dtype.contains("float32") {
        read_as!(f32)
    } else if dtype.contains("float64") {
        read_as!(f64)
    } else if dtype.contains("int16") {
        read_as!(i16)
    } else if dtype.contains("int32") {
        read_as!(i32)
    } else if dtype.contains("int64") {
        read_as!(i64)
    } else {
        Err(ZarrError::Layout(format!(
            "{path} is stored as {dtype}, which this reader does not handle"
        )))
    }
}

/// Confirms a coordinate axis has the expected length, starts where it should
/// and steps the right way.
///
/// The sign of `delta` is the point: it is what decides whether a decoded
/// chunk can go into a GRIB section 7 as it is, or has to be flipped first.
pub fn check_axis(
    name: &str,
    values: &[f32],
    expected_len: usize,
    first: f32,
    delta: f32,
) -> Result<()> {
    if values.len() != expected_len {
        return Err(ZarrError::Layout(format!(
            "{name} has {} values, expected {expected_len}",
            values.len()
        )));
    }
    let Some(&start) = values.first() else {
        return Err(ZarrError::Layout(format!("{name} is empty")));
    };
    if (start - first).abs() > 1e-3 {
        return Err(ZarrError::Layout(format!(
            "{name} starts at {start}, expected {first}"
        )));
    }
    let Some(&second) = values.get(1) else {
        return Err(ZarrError::Layout(format!("{name} has a single value")));
    };
    if ((second - start) - delta).abs() > 1e-3 {
        return Err(ZarrError::Layout(format!(
            "{name} steps by {}, expected {delta}; the grid is not in the assumed order",
            second - start
        )));
    }
    let expected_last = first + delta * (values.len() - 1) as f32;
    let last = values[values.len() - 1];
    if (last - expected_last).abs() > 1e-3 {
        return Err(ZarrError::Layout(format!(
            "{name} ends at {last}, expected {expected_last}"
        )));
    }
    Ok(())
}

/// Reads a whole time axis and converts it to hours since the Unix epoch.
///
/// The axis is read in full, once, rather than assuming the steps are
/// contiguous: the index of a given hour then comes from a binary search over
/// real values instead of arithmetic that would be wrong without saying so.
/// Integer and floating axes are both accepted, as long as every value is a
/// whole number of hours.
pub fn read_time_axis(store: &ReadableStorage, path: &str, expected_len: u64) -> Result<Vec<i64>> {
    let array = open_array(store, path)?;
    if array.shape() != [expected_len] {
        return Err(ZarrError::Layout(format!(
            "the time axis has shape {:?} but the data arrays have {expected_len} steps",
            array.shape()
        )));
    }

    let units = array
        .attributes()
        .get("units")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ZarrError::Layout("the time axis has no units attribute".into()))?
        .to_string();
    let epoch = parse_epoch(&units)?;
    let base = epoch.hours_since_unix_epoch();

    // `DataType` is opaque here; its name is the stable thing to dispatch on.
    let dtype = format!("{:?}", array.data_type()).to_ascii_lowercase();
    #[allow(clippy::single_range_in_vec_init, reason = "one range per dimension")]
    let raw: Vec<f64> = if dtype.contains("int64") {
        array
            .retrieve_array_subset::<Vec<i64>>(&[0..expected_len])
            .map_err(read_err("the time axis"))?
            .into_iter()
            .map(|h| h as f64)
            .collect()
    } else if dtype.contains("int32") {
        array
            .retrieve_array_subset::<Vec<i32>>(&[0..expected_len])
            .map_err(read_err("the time axis"))?
            .into_iter()
            .map(f64::from)
            .collect()
    } else if dtype.contains("float32") {
        array
            .retrieve_array_subset::<Vec<f32>>(&[0..expected_len])
            .map_err(read_err("the time axis"))?
            .into_iter()
            .map(f64::from)
            .collect()
    } else if dtype.contains("float64") {
        array
            .retrieve_array_subset::<Vec<f64>>(&[0..expected_len])
            .map_err(read_err("the time axis"))?
    } else {
        return Err(ZarrError::Layout(format!(
            "the time axis is stored as {dtype}, which this reader does not handle"
        )));
    };

    let mut times = Vec::with_capacity(raw.len());
    for (i, h) in raw.into_iter().enumerate() {
        if !h.is_finite() || h.fract() != 0.0 {
            return Err(ZarrError::Layout(format!(
                "time axis entry {i} is {h}, not a whole number of hours"
            )));
        }
        times.push(base + h as i64);
    }
    if times.windows(2).any(|w| w[1] <= w[0]) {
        return Err(ZarrError::Layout(
            "the time axis is not strictly increasing".into(),
        ));
    }
    Ok(times)
}

/// Parses the CF `units` string, which must be in whole hours.
///
/// Anything else -- minutes, seconds, a different calendar's epoch -- would
/// change what every index means, so it is rejected rather than guessed at.
pub fn parse_epoch(units: &str) -> Result<Utc> {
    let rest = units.strip_prefix("hours since ").ok_or_else(|| {
        ZarrError::Layout(format!(
            "the time axis is measured in {units:?}; this reader requires whole hours"
        ))
    })?;
    // A trailing UTC offset is fine; any other offset would shift the epoch.
    let rest = rest.trim();
    let rest = rest
        .strip_suffix("+00:00")
        .or_else(|| rest.strip_suffix('Z'))
        .unwrap_or(rest);
    Utc::parse(rest.trim())
        .ok_or_else(|| ZarrError::Layout(format!("could not parse the time epoch {rest:?}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_era5_epoch_parses() {
        assert_eq!(
            parse_epoch("hours since 1959-01-01 00:00:00").expect("parses"),
            Utc {
                year: 1959,
                month: 1,
                day: 1,
                hour: 0
            }
        );
    }

    /// GlobCurrent writes its epoch with an explicit zero offset.
    #[test]
    fn the_globcurrent_epoch_parses() {
        assert_eq!(
            parse_epoch("hours since 1950-01-01T00:00:00+00:00").expect("parses"),
            Utc {
                year: 1950,
                month: 1,
                day: 1,
                hour: 0
            }
        );
    }

    /// A store in minutes would index correctly and mean something else
    /// entirely, so it has to be refused.
    #[test]
    fn units_other_than_hours_are_refused() {
        for units in [
            "minutes since 1959-01-01 00:00:00",
            "seconds since 1970-01-01 00:00:00",
            "days since 1900-01-01 00:00:00",
        ] {
            assert!(parse_epoch(units).is_err(), "{units}");
        }
    }

    #[test]
    fn a_descending_latitude_axis_is_accepted() {
        let lat: Vec<f32> = (0..721).map(|j| 90.0 - j as f32 * 0.25).collect();
        check_axis("latitude", &lat, 721, 90.0, -0.25).expect("north to south");
    }

    /// The check exists for exactly this case: an ascending latitude axis
    /// would decode fine and write an upside-down file.
    #[test]
    fn an_ascending_latitude_axis_is_refused_when_descending_is_expected() {
        let lat: Vec<f32> = (0..721).map(|j| -90.0 + j as f32 * 0.25).collect();
        let err = check_axis("latitude", &lat, 721, 90.0, -0.25).expect_err("must refuse");
        assert!(format!("{err}").contains("starts at"), "{err}");
    }

    #[test]
    fn a_longitude_axis_centred_on_zero_is_refused_when_zero_based_is_expected() {
        let lon: Vec<f32> = (0..1440).map(|i| -180.0 + i as f32 * 0.25).collect();
        assert!(check_axis("longitude", &lon, 1440, 0.0, 0.25).is_err());
    }

    #[test]
    fn an_axis_with_the_wrong_length_is_refused() {
        let lat: Vec<f32> = (0..100).map(|j| 90.0 - j as f32 * 0.25).collect();
        let err = check_axis("latitude", &lat, 721, 90.0, -0.25).expect_err("must refuse");
        assert!(format!("{err}").contains("expected 721"), "{err}");
    }
}

//! Zarr V3 writer used by the project exporter.
//!
//! The data array deliberately has one parameter dimension instead of four
//! child arrays. This keeps the four components in the same compressed chunk,
//! which is useful for consumers that need a vector at a point. The writer is
//! kept here with the Zarr reader so the format details have one owner.

use std::path::Path;
use std::sync::Arc;

pub use half::f16;
use serde_json::{Map, Value, json};
use zarrs::array::codec::ZstdCodec;
use zarrs::array::{Array, ArrayBuilder, ZARR_NAN_F16, data_type};
use zarrs::filesystem::FilesystemStore;
use zarrs::group::GroupBuilder;

use crate::error::{Result, ZarrError};

/// The fixed parameter order in every data chunk.
pub const PARAMETERS: [&str; 4] = [
    "u10m_wind",
    "v10m_wind",
    "u_total_surface_current",
    "v_total_surface_current",
];

/// Zarr array shape and regular chunk shape for a project.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Layout {
    /// `[time, parameter, latitude, longitude]` shape.
    pub shape: [u64; 4],
    /// Grid spacing in degrees, the same along both axes.
    pub resolution_degrees: f64,
    /// Hours between consecutive time steps.
    pub step_hours: u32,
    /// Regular chunk shape. The last chunks may be smaller at the edges.
    pub chunk_shape: [u64; 4],
    /// Number of time steps in three days.
    pub time_chunk_steps: u64,
    /// Number of grid points spanning ten degrees at this resolution.
    pub spatial_chunk_points: u64,
}

impl Layout {
    /// Builds the requested 72-hour by 10-degree tiling.
    #[must_use]
    pub fn new(resolution_degrees: f64, steps: u32, step_hours: u32, ni: u32, nj: u32) -> Self {
        let spatial = (10.0 / resolution_degrees).round().max(1.0) as u64;
        let time = (72 / step_hours.max(1)).max(1) as u64;
        Self {
            shape: [
                u64::from(steps),
                PARAMETERS.len() as u64,
                u64::from(nj),
                u64::from(ni),
            ],
            resolution_degrees,
            step_hours,
            chunk_shape: [time, PARAMETERS.len() as u64, spatial, spatial],
            time_chunk_steps: time,
            spatial_chunk_points: spatial,
        }
    }

    /// Number of chunks in each dimension.
    #[must_use]
    pub fn chunk_grid_shape(self) -> [u64; 4] {
        std::array::from_fn(|axis| self.shape[axis].div_ceil(self.chunk_shape[axis]))
    }
}

/// Where the lattice starts and which way it runs: the first row is the north
/// pole and the first column the prime meridian, longitude increasing eastward
/// through `[0, 360)` — the GRIB scanning order, so the two exports of one
/// project put every cell in the same place.
pub const LATITUDE_START: f64 = 90.0;
/// See [`LATITUDE_START`].
pub const LONGITUDE_START: f64 = 0.0;

/// A filesystem-backed Zarr V3 writer.
#[derive(Debug)]
pub struct Writer {
    array: Array<FilesystemStore>,
    layout: Layout,
}

impl Writer {
    /// Creates a Zarr V3 group and its four-parameter Float16 data array.
    ///
    /// `reference_time` is the validity of time index 0 as an ISO 8601 UTC
    /// instant (`2026-09-02T00:00:00Z`); step `t` is valid `t * step_hours`
    /// hours later. Without it and the lattice attributes the array is a bare
    /// block of numbers: a consumer could not place a cell on the earth or on
    /// the clock.
    pub fn create(
        path: &Path,
        layout: Layout,
        reference_time: &str,
        start_unix_s: Option<i64>,
    ) -> Result<Self> {
        let store = Arc::new(
            FilesystemStore::new(path).map_err(|error| ZarrError::Write(error.to_string()))?,
        );

        let mut root_attributes = Map::new();
        root_attributes.insert("created_by".into(), Value::String("VectorEffects".into()));
        root_attributes.insert("zarr_export_version".into(), Value::String("1".into()));
        root_attributes.insert("parameter_order".into(), json!(PARAMETERS));
        root_attributes.insert("land_masked".into(), Value::Bool(true));
        root_attributes.insert("missing_value".into(), Value::String("NaN".into()));
        root_attributes.insert("time_chunk_hours".into(), json!(72));
        root_attributes.insert("spatial_chunk_degrees".into(), json!(10));
        if let Some(start) = start_unix_s {
            root_attributes.insert("start_unix_s".into(), json!(start));
        }
        GroupBuilder::new()
            .attributes(root_attributes)
            .build(store.clone(), "/")
            .map_err(|error| ZarrError::Write(error.to_string()))?
            .store_metadata()
            .map_err(|error| ZarrError::Write(error.to_string()))?;

        let mut attributes = Map::new();
        attributes.insert("parameter_order".into(), json!(PARAMETERS));
        attributes.insert(
            "parameter_units".into(),
            json!(["m s-1", "m s-1", "m s-1", "m s-1"]),
        );
        attributes.insert("land_masked".into(), Value::Bool(true));
        attributes.insert("missing_value".into(), Value::String("NaN".into()));
        attributes.insert(
            "reference_time".into(),
            Value::String(reference_time.into()),
        );
        attributes.insert("step_hours".into(), json!(layout.step_hours));
        attributes.insert("latitude_start".into(), json!(LATITUDE_START));
        attributes.insert("latitude_step".into(), json!(-layout.resolution_degrees));
        attributes.insert("longitude_start".into(), json!(LONGITUDE_START));
        attributes.insert("longitude_step".into(), json!(layout.resolution_degrees));

        let mut builder = ArrayBuilder::new(
            layout.shape.to_vec(),
            layout.chunk_shape.to_vec(),
            data_type::float16(),
            ZARR_NAN_F16,
        );
        builder
            .bytes_to_bytes_codecs(vec![Arc::new(ZstdCodec::new(3, false))])
            .dimension_names(Some(["time", "parameter", "latitude", "longitude"]))
            .attributes(attributes);
        let array = builder
            .build(store, "/data")
            .map_err(|error| ZarrError::Write(error.to_string()))?;
        array
            .store_metadata()
            .map_err(|error| ZarrError::Write(error.to_string()))?;

        Ok(Self { array, layout })
    }

    /// Stores one complete chunk, `[time, parameter, row, column]` in row-major
    /// order and always the regular chunk shape: at the array's edge the cells
    /// past the last step, row or column are padding and should hold NaN.
    /// NaN is the fill value, so it preserves the land mask rather than
    /// becoming zero.
    pub fn write_chunk(&self, indices: &[u64], values: &[f16]) -> Result<()> {
        let chunk_shape = self
            .array
            .chunk_shape(indices)
            .map_err(|error| ZarrError::Write(error.to_string()))?;
        let expected = chunk_shape
            .iter()
            .try_fold(1usize, |count, size| count.checked_mul(size.get() as usize))
            .ok_or_else(|| ZarrError::Write("chunk size overflow".into()))?;
        if values.len() != expected {
            return Err(ZarrError::Write(format!(
                "chunk {:?} has {} values; expected {}",
                indices,
                values.len(),
                expected
            )));
        }
        self.array
            .store_chunk(indices, values.to_vec())
            .map_err(|error| ZarrError::Write(error.to_string()))
    }

    /// The shape/chunk metadata used by this writer.
    #[must_use]
    pub const fn layout(&self) -> Layout {
        self.layout
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_uses_72_hours_and_ten_degree_chunks() {
        let hourly = Layout::new(0.25, 100, 1, 1440, 721);
        assert_eq!(hourly.time_chunk_steps, 72);
        assert_eq!(hourly.spatial_chunk_points, 40);
        assert_eq!(hourly.chunk_shape, [72, 4, 40, 40]);

        let three_hourly = Layout::new(0.25, 100, 3, 1440, 721);
        assert_eq!(three_hourly.chunk_shape, [24, 4, 40, 40]);
        let six_hourly = Layout::new(0.25, 100, 6, 1440, 721);
        assert_eq!(six_hourly.chunk_shape, [12, 4, 40, 40]);
    }

    #[test]
    fn layout_has_parameter_axis_inside_each_chunk() {
        let layout = Layout::new(1.0, 3, 24, 360, 181);
        assert_eq!(layout.shape[1], 4);
        assert_eq!(layout.chunk_shape[1], 4);
        assert_eq!(PARAMETERS.len(), 4);
    }
}

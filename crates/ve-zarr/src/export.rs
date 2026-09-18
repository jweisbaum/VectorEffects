//! Zarr V3 writer used by the project exporter (spec.md 12.3).
//!
//! The store is the routing layout: one Float16 `data` array with dimensions
//! `(time, param, latitude, longitude)`, the four components together in
//! every chunk, beside the `time`, `param`, `latitude` and `longitude`
//! coordinate arrays. Inner chunks are three days by ten degrees by ten
//! degrees; they are gathered by the `sharding_indexed` codec into shards
//! laid out on a **rectilinear** chunk grid whose boxes are ocean basins
//! ([`LAT_EDGES_DEG`], [`LON_EDGES_DEG`]), so a passage reads a handful of
//! files rather than hundreds.
//!
//! The metadata and the shard files are written here rather than through
//! `zarrs`. A shard is a concatenation of compressed inner chunks with an
//! index at the end, and written by hand it can be *streamed*: `zarrs` takes
//! a whole shard in one call, and the largest at 0.1° hourly is 400 MB of
//! Float16 before its encoded copy. Reading it back through `zarrs` — and
//! through zarr-python, which made the store this one is modelled on — is
//! what the tests do instead.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

pub use half::f16;
use serde_json::{Value, json};

use crate::error::{Result, ZarrError};

/// The `param` coordinate, and the order of the four components in a chunk.
pub const PARAMETERS: [&str; 4] = ["u10", "v10", "ucur", "vcur"];

/// What each [`PARAMETERS`] entry is, in the same order.
pub const PARAMETER_LONG_NAMES: [&str; 4] = [
    "10 metre U wind component",
    "10 metre V wind component",
    "eastward sea water velocity",
    "northward sea water velocity",
];

/// Shard edges along latitude, north to south, in degrees.
///
/// Every edge sits on a ten-degree line, so the ten-degree inner chunks
/// divide every shard — which the sharding codec requires.
pub const LAT_EDGES_DEG: [i32; 5] = [90, 70, 0, -60, -90];
/// Shard edges along longitude, west to east, in degrees.
pub const LON_EDGES_DEG: [i32; 6] = [-180, -100, -60, 20, 120, 180];
/// Names of the latitude bands between [`LAT_EDGES_DEG`].
pub const LAT_BAND_NAMES: [&str; 4] = ["arctic", "northern", "southern", "southern_ocean"];
/// Names of the longitude bands between [`LON_EDGES_DEG`].
pub const LON_BAND_NAMES: [&str; 5] = [
    "pacific_east",
    "americas",
    "atlantic",
    "indian",
    "pacific_west",
];

/// Hours of forecast in one chunk along time, whatever the project's step.
pub const HOURS_PER_CHUNK: u32 = 72;
/// Degrees spanned by one inner chunk along latitude and longitude.
pub const TILE_DEGREES: u32 = 10;

/// The name of the shard at a latitude band and a longitude band: the basin
/// where it has one, and the two band names joined where it does not.
#[must_use]
pub fn shard_name(lat_band: usize, lon_band: usize) -> String {
    let (lat, lon) = (LAT_BAND_NAMES[lat_band], LON_BAND_NAMES[lon_band]);
    let named = match (lat, lon) {
        ("northern", "atlantic") => "north_atlantic",
        ("southern", "atlantic") => "south_atlantic",
        ("northern", "americas") => "caribbean",
        ("southern", "indian") => "indian_ocean",
        ("northern", "indian") => "arabian_sea_bay_of_bengal",
        ("northern", "pacific_west") => "north_west_pacific",
        ("southern", "pacific_west") => "south_west_pacific",
        ("northern", "pacific_east") => "north_east_pacific",
        ("southern", "pacific_east") => "south_east_pacific",
        ("southern", "americas") => "south_america_west_atlantic",
        _ => return format!("{lat}_{lon}"),
    };
    named.to_owned()
}

/// The shape of the store for one project.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Layout {
    /// `[time, param, latitude, longitude]`.
    ///
    /// Latitude runs from 90° down to one step short of −90°: the south pole
    /// row is left out so that ten-degree chunks divide the axis. Longitude
    /// runs from −180° to one step short of 180°.
    pub shape: [u64; 4],
    /// Grid spacing in micro-degrees, the same along both axes.
    pub micro_degrees: u32,
    /// Hours between consecutive time steps.
    pub step_hours: u32,
    /// Time steps in three days: 72 hourly, 24 three-hourly, 12 six-hourly.
    pub time_chunk_steps: u64,
    /// Grid points spanning ten degrees.
    pub tile: u64,
    /// Rows in each latitude band's shards, north to south.
    pub lat_shards: [u64; 4],
    /// Columns in each longitude band's shards, west to east.
    pub lon_shards: [u64; 5],
}

impl Layout {
    /// The layout for a grid spacing, a step count and a step length.
    ///
    /// Refused when ten degrees is not a whole number of grid points: the
    /// inner chunks could not tile the shards.
    pub fn new(micro_degrees: u32, steps: u32, step_hours: u32) -> Result<Self> {
        let tile_micro = TILE_DEGREES * 1_000_000;
        if micro_degrees == 0 || !tile_micro.is_multiple_of(micro_degrees) {
            return Err(ZarrError::Write(format!(
                "a grid spacing of {micro_degrees} micro-degrees does not divide ten degrees"
            )));
        }
        let tile = u64::from(tile_micro / micro_degrees);
        let points = |degrees: i32| u64::from(degrees.unsigned_abs()) * tile / 10;
        let lat_shards =
            std::array::from_fn(|band| points(LAT_EDGES_DEG[band] - LAT_EDGES_DEG[band + 1]));
        let lon_shards =
            std::array::from_fn(|band| points(LON_EDGES_DEG[band + 1] - LON_EDGES_DEG[band]));
        Ok(Self {
            shape: [
                u64::from(steps),
                PARAMETERS.len() as u64,
                points(180),
                points(360),
            ],
            micro_degrees,
            step_hours,
            time_chunk_steps: u64::from((HOURS_PER_CHUNK / step_hours.max(1)).max(1)),
            tile,
            lat_shards,
            lon_shards,
        })
    }

    /// The inner chunk shape, `[time, param, latitude, longitude]`.
    #[must_use]
    pub fn chunk_shape(self) -> [u64; 4] {
        [
            self.time_chunk_steps,
            PARAMETERS.len() as u64,
            self.tile,
            self.tile,
        ]
    }

    /// Values in one inner chunk.
    #[must_use]
    pub fn chunk_len(self) -> usize {
        self.chunk_shape().iter().product::<u64>() as usize
    }

    /// Latitude of a row, in degrees.
    #[must_use]
    pub fn latitude(self, row: u64) -> f64 {
        (90_000_000 - row as i64 * i64::from(self.micro_degrees)) as f64 / 1e6
    }

    /// Longitude of a column, in degrees.
    #[must_use]
    pub fn longitude(self, column: u64) -> f64 {
        (column as i64 * i64::from(self.micro_degrees) - 180_000_000) as f64 / 1e6
    }

    /// Which band a tile falls in, and its position within that band.
    fn band_of(shards: &[u64], tile: u64, index: u64) -> Option<(usize, u64)> {
        let mut first = 0;
        for (band, points) in shards.iter().enumerate() {
            let tiles = points / tile;
            if index < first + tiles {
                return Some((band, index - first));
            }
            first += tiles;
        }
        None
    }
}

/// One shard file being filled: the compressed inner chunks so far, and
/// where each one landed.
#[derive(Debug)]
struct Shard {
    file: BufWriter<File>,
    path: PathBuf,
    /// `(offset, length)` per inner chunk in C order; `None` is a chunk that
    /// holds nothing but the fill value and is left out.
    index: Vec<Option<(u64, u64)>>,
    written: u64,
}

/// A filesystem-backed Zarr V3 writer.
#[derive(Debug)]
pub struct Writer {
    root: PathBuf,
    layout: Layout,
    /// The shards of the time chunk being written, by band.
    open: Vec<((usize, usize), Shard)>,
    /// The time chunk those belong to.
    time_index: u64,
}

/// An index entry for an inner chunk that was not stored.
const MISSING: u64 = u64::MAX;

impl Writer {
    /// Creates the group, the `data` array's metadata and the four
    /// coordinate arrays.
    ///
    /// `reference_time` is the validity of time index 0, as
    /// `2026-09-02T00:00:00`; `time` counts hours since it. `title` and
    /// `source` are the group's attributes of those names.
    pub fn create(
        path: &Path,
        layout: Layout,
        title: &str,
        source: &str,
        reference_time: &str,
    ) -> Result<Self> {
        let write = |relative: &str, bytes: &[u8]| -> Result<()> {
            let file = path.join(relative);
            if let Some(parent) = file.parent() {
                std::fs::create_dir_all(parent).map_err(failed(parent))?;
            }
            std::fs::write(&file, bytes).map_err(failed(&file))
        };
        let document = |value: &Value| -> Result<Vec<u8>> {
            serde_json::to_vec_pretty(value).map_err(|error| ZarrError::Write(error.to_string()))
        };

        write("zarr.json", &document(&group_metadata(title, source))?)?;
        write("data/zarr.json", &document(&data_metadata(layout))?)?;

        let long_names: Value = PARAMETERS
            .iter()
            .zip(PARAMETER_LONG_NAMES)
            .map(|(name, long)| ((*name).to_owned(), Value::String(long.to_owned())))
            .collect::<serde_json::Map<_, _>>()
            .into();
        let [steps, _, rows, columns] = layout.shape;
        let coordinates: [(&str, Value, Value, Value, Vec<u8>); 4] = [
            (
                "time",
                json!("int64"),
                json!(0),
                json!({
                    "units": format!("hours since {reference_time}"),
                    "calendar": "proleptic_gregorian"
                }),
                (0..steps)
                    .flat_map(|t| (t as i64 * i64::from(layout.step_hours)).to_le_bytes())
                    .collect(),
            ),
            (
                "param",
                json!({"name": "fixed_length_utf32", "configuration": {"length_bytes": 16}}),
                json!(""),
                json!({ "long_names": long_names }),
                PARAMETERS.iter().flat_map(|name| utf32(name, 4)).collect(),
            ),
            (
                "latitude",
                json!("float32"),
                json!(0.0),
                json!({"units": "degrees_north"}),
                (0..rows)
                    .flat_map(|row| (layout.latitude(row) as f32).to_le_bytes())
                    .collect(),
            ),
            (
                "longitude",
                json!("float32"),
                json!(0.0),
                json!({"units": "degrees_east"}),
                (0..columns)
                    .flat_map(|column| (layout.longitude(column) as f32).to_le_bytes())
                    .collect(),
            ),
        ];
        for (name, data_type, fill_value, attributes, bytes) in coordinates {
            let length = match name {
                "time" => steps,
                "param" => PARAMETERS.len() as u64,
                "latitude" => rows,
                _ => columns,
            };
            let metadata = coordinate_metadata(name, length, data_type, fill_value, attributes);
            write(&format!("{name}/zarr.json"), &document(&metadata)?)?;
            // A project with no steps has an empty axis, and an empty array
            // has no chunk to store.
            if length > 0 {
                // Level 0 is zstd's own default, which is what the codec's
                // `"level": 0` says.
                let packed = zstd::bulk::compress(&bytes, 0)
                    .map_err(|error| ZarrError::Write(error.to_string()))?;
                write(&format!("{name}/c/0"), &packed)?;
            }
        }

        Ok(Self {
            root: path.to_path_buf(),
            layout,
            open: Vec::new(),
            time_index: 0,
        })
    }

    /// Stores one inner chunk, `[time, param, row, column]` in row-major
    /// order and always the full [`Layout::chunk_shape`]: past the project's
    /// last step the cells are padding and hold NaN.
    ///
    /// `time_index`, `tile_row` and `tile_column` count inner chunks from the
    /// start of the array. A chunk that is NaN throughout is the fill value
    /// and is not stored, and a shard none of whose chunks were stored is not
    /// a file — what zarr-python does, and what keeps a regional project's
    /// store the size of the region. Returns whether it was stored.
    ///
    /// Every chunk of one time index must be written, and
    /// [`Self::finish_time`] called, before the next begins.
    pub fn write_chunk(
        &mut self,
        time_index: u64,
        tile_row: u64,
        tile_column: u64,
        values: &[f16],
    ) -> Result<bool> {
        let layout = self.layout;
        if values.len() != layout.chunk_len() {
            return Err(ZarrError::Write(format!(
                "chunk [{time_index}, 0, {tile_row}, {tile_column}] has {} values; expected {}",
                values.len(),
                layout.chunk_len()
            )));
        }
        if !self.open.is_empty() && time_index != self.time_index {
            return Err(ZarrError::Write(format!(
                "time chunk {} was begun before {} was finished",
                time_index, self.time_index
            )));
        }
        let outside = || ZarrError::Write(format!("no tile [{tile_row}, {tile_column}]"));
        let (lat_band, local_row) =
            Layout::band_of(&layout.lat_shards, layout.tile, tile_row).ok_or_else(outside)?;
        let (lon_band, local_column) =
            Layout::band_of(&layout.lon_shards, layout.tile, tile_column).ok_or_else(outside)?;
        if values.iter().all(|value| value.is_nan()) {
            return Ok(false);
        }
        self.time_index = time_index;

        let tiles_across = layout.lon_shards[lon_band] / layout.tile;
        let tiles_down = layout.lat_shards[lat_band] / layout.tile;
        let position = match self
            .open
            .iter()
            .position(|(band, _)| *band == (lat_band, lon_band))
        {
            Some(position) => position,
            None => {
                let file = self
                    .root
                    .join(format!("data/c/{time_index}/0/{lat_band}/{lon_band}"));
                if let Some(parent) = file.parent() {
                    std::fs::create_dir_all(parent).map_err(failed(parent))?;
                }
                let shard = Shard {
                    file: BufWriter::new(File::create(&file).map_err(failed(&file))?),
                    path: file,
                    index: vec![None; (tiles_down * tiles_across) as usize],
                    written: 0,
                };
                self.open.push(((lat_band, lon_band), shard));
                self.open.len() - 1
            }
        };
        let shard = &mut self.open[position].1;

        // The `bytes` codec, little-endian, then `zstd` at level 5.
        let raw: Vec<u8> = values
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect();
        let packed = zstd::bulk::compress(&raw, ZSTD_LEVEL)
            .map_err(|error| ZarrError::Write(error.to_string()))?;
        shard.file.write_all(&packed).map_err(failed(&shard.path))?;
        shard.index[(local_row * tiles_across + local_column) as usize] =
            Some((shard.written, packed.len() as u64));
        shard.written += packed.len() as u64;
        Ok(true)
    }

    /// Closes the shards of the time chunk just written, each with its index:
    /// an offset and a length per inner chunk as little-endian `u64`s, then
    /// the CRC-32C of those bytes (`index_location: end`).
    pub fn finish_time(&mut self) -> Result<()> {
        for (_, mut shard) in self.open.drain(..) {
            let mut index = Vec::with_capacity(shard.index.len() * 16 + 4);
            for entry in &shard.index {
                let (offset, length) = entry.unwrap_or((MISSING, MISSING));
                index.extend_from_slice(&offset.to_le_bytes());
                index.extend_from_slice(&length.to_le_bytes());
            }
            let checksum = crc32c::crc32c(&index);
            index.extend_from_slice(&checksum.to_le_bytes());
            shard.file.write_all(&index).map_err(failed(&shard.path))?;
            shard.file.flush().map_err(failed(&shard.path))?;
        }
        Ok(())
    }

    /// The layout this writer was created with.
    #[must_use]
    pub const fn layout(&self) -> Layout {
        self.layout
    }
}

/// The inner chunks' compression level.
const ZSTD_LEVEL: i32 = 5;

fn failed(path: &Path) -> impl Fn(std::io::Error) -> ZarrError + '_ {
    move |error| ZarrError::Write(format!("{}: {error}", path.display()))
}

/// A string as `fixed_length_utf32` stores it: one little-endian `u32` per
/// code point, padded with zeros to `length` code points.
fn utf32(text: &str, length: usize) -> Vec<u8> {
    let mut points: Vec<u32> = text.chars().map(u32::from).collect();
    points.resize(length, 0);
    points.into_iter().flat_map(u32::to_le_bytes).collect()
}

fn bytes_codec() -> Value {
    json!({"name": "bytes", "configuration": {"endian": "little"}})
}

fn zstd_codec(level: i32) -> Value {
    json!({"name": "zstd", "configuration": {"level": level, "checksum": false}})
}

fn group_metadata(title: &str, source: &str) -> Value {
    let shards: Vec<Value> = (0..LAT_BAND_NAMES.len())
        .flat_map(|i| (0..LON_BAND_NAMES.len()).map(move |j| (i, j)))
        .map(|(i, j)| {
            json!({
                "name": shard_name(i, j),
                "lat_band": LAT_BAND_NAMES[i],
                "lon_band": LON_BAND_NAMES[j],
                "lat_deg": [LAT_EDGES_DEG[i], LAT_EDGES_DEG[i + 1]],
                "lon_deg": [LON_EDGES_DEG[j], LON_EDGES_DEG[j + 1]],
                "shard_index_lat_lon": [i, j]
            })
        })
        .collect();
    json!({
        "attributes": {
            "title": title,
            "source": source,
            "conventions": "param is a dimension; data array dims are (time, param, latitude, longitude)",
            "land_mask": "NaN where the project defines no field: an uncovered or masked cell, and both components of a kind of field the project does not hold",
            "shard_lat_edges_deg": LAT_EDGES_DEG,
            "shard_lon_edges_deg": LON_EDGES_DEG,
            "shard_lat_bands": LAT_BAND_NAMES,
            "shard_lon_bands": LON_BAND_NAMES,
            "shards": shards
        },
        "zarr_format": 3,
        "node_type": "group"
    })
}

fn data_metadata(layout: Layout) -> Value {
    let long_names: serde_json::Map<String, Value> = PARAMETERS
        .iter()
        .zip(PARAMETER_LONG_NAMES)
        .map(|(name, long)| ((*name).to_owned(), Value::String(long.to_owned())))
        .collect();
    json!({
        "shape": layout.shape,
        "data_type": "float16",
        "chunk_grid": {
            "name": "rectilinear",
            "configuration": {
                "kind": "inline",
                "chunk_shapes": [
                    layout.time_chunk_steps,
                    PARAMETERS.len(),
                    layout.lat_shards,
                    layout.lon_shards
                ]
            }
        },
        "chunk_key_encoding": {"name": "default", "configuration": {"separator": "/"}},
        "fill_value": "NaN",
        "codecs": [{
            "name": "sharding_indexed",
            "configuration": {
                "chunk_shape": layout.chunk_shape(),
                "codecs": [bytes_codec(), zstd_codec(ZSTD_LEVEL)],
                "index_codecs": [bytes_codec(), {"name": "crc32c"}],
                "index_location": "end"
            }
        }],
        "attributes": {
            "units": "m s-1",
            "param_long_names": long_names,
            "_FillValue": "NaN"
        },
        "dimension_names": ["time", "param", "latitude", "longitude"],
        "zarr_format": 3,
        "node_type": "array",
        "storage_transformers": []
    })
}

fn coordinate_metadata(
    name: &str,
    length: u64,
    data_type: Value,
    fill_value: Value,
    attributes: Value,
) -> Value {
    json!({
        "shape": [length],
        "data_type": data_type,
        "chunk_grid": {
            "name": "regular",
            // A chunk may not be empty, even where the axis is.
            "configuration": {"chunk_shape": [length.max(1)]}
        },
        "chunk_key_encoding": {"name": "default", "configuration": {"separator": "/"}},
        "fill_value": fill_value,
        "codecs": [bytes_codec(), zstd_codec(0)],
        "attributes": attributes,
        "dimension_names": [name],
        "zarr_format": 3,
        "node_type": "array",
        "storage_transformers": []
    })
}

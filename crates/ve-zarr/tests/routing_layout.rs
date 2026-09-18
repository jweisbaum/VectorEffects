//! The Zarr export against `routing_test`, the store it is modelled on
//! (spec.md 12.3), and against `zarrs` as the reader.
//!
//! `fixtures/routing_test` holds that store's metadata documents and
//! coordinate chunks as zarr-python wrote them. At the same resolution, step
//! and length the export's documents must be the same text, except for the
//! values that say what the data *is* — the title, the source, the mask's
//! description and the parameters' long names.

#![allow(clippy::expect_used, clippy::unwrap_used)]
#![allow(
    clippy::single_range_in_vec_init,
    reason = "a one-dimensional zarrs ArraySubset is built from a slice of one range"
)]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::Value;
use ve_zarr::export::{Layout, PARAMETER_LONG_NAMES, PARAMETERS, Writer, f16};

fn fixture(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/routing_test")
        .join(relative)
}

fn text(path: &Path) -> String {
    std::fs::read_to_string(path).expect("read")
}

/// January 2012, hourly, at a quarter of a degree: `routing_test` itself.
fn routing_test_layout() -> Layout {
    Layout::new(250_000, 744, 1).expect("layout")
}

fn long_names() -> Value {
    PARAMETERS
        .iter()
        .zip(PARAMETER_LONG_NAMES)
        .map(|(name, long)| ((*name).to_owned(), Value::String(long.to_owned())))
        .collect::<serde_json::Map<_, _>>()
        .into()
}

/// The reference document with the descriptive values swapped for ours, as
/// text: key order and layout are compared, not only content.
fn expected(relative: &str, patch: impl FnOnce(&mut Value)) -> String {
    let mut value: Value = serde_json::from_str(&text(&fixture(relative))).expect("json");
    patch(&mut value);
    serde_json::to_string_pretty(&value).expect("pretty")
}

#[test]
fn the_metadata_is_routing_tests_own() {
    let out = tempfile::tempdir().expect("tempdir");
    Writer::create(
        out.path(),
        routing_test_layout(),
        "a title",
        "a source",
        "2012-01-01T00:00:00",
    )
    .expect("create");

    // The two grid axes describe nothing of ours: the file is the same file.
    for axis in ["latitude/zarr.json", "longitude/zarr.json"] {
        assert_eq!(text(&out.path().join(axis)), text(&fixture(axis)), "{axis}");
    }
    assert_eq!(
        text(&out.path().join("time/zarr.json")),
        text(&fixture("time/zarr.json")),
        "the same reference time gives the same units"
    );
    assert_eq!(
        text(&out.path().join("param/zarr.json")),
        expected("param/zarr.json", |v| v["attributes"]["long_names"] =
            long_names())
    );
    assert_eq!(
        text(&out.path().join("data/zarr.json")),
        expected("data/zarr.json", |v| {
            v["attributes"]["param_long_names"] = long_names();
        })
    );
    let ours: Value = serde_json::from_str(&text(&out.path().join("zarr.json"))).expect("json");
    assert_eq!(
        text(&out.path().join("zarr.json")),
        expected("zarr.json", |v| {
            v["attributes"]["title"] = "a title".into();
            v["attributes"]["source"] = "a source".into();
            v["attributes"]["land_mask"] = ours["attributes"]["land_mask"].clone();
        })
    );
}

#[test]
fn the_coordinates_hold_routing_tests_values() {
    let out = tempfile::tempdir().expect("tempdir");
    Writer::create(
        out.path(),
        routing_test_layout(),
        "",
        "",
        "2012-01-01T00:00:00",
    )
    .expect("create");
    for axis in ["time", "param", "latitude", "longitude"] {
        let chunk = format!("{axis}/c/0");
        let unpack = |path: &Path| {
            zstd::stream::decode_all(std::fs::File::open(path).expect("open")).expect("zstd")
        };
        assert_eq!(
            unpack(&out.path().join(&chunk)),
            unpack(&fixture(&chunk)),
            "{axis}"
        );
    }
}

/// Three days of steps in a chunk whatever the step, and the basins scaled
/// with the grid: the two things that are allowed to differ.
#[test]
fn a_chunk_is_three_days_and_a_shard_is_a_basin_at_any_resolution() {
    let steps = |step_hours| Layout::new(250_000, 100, step_hours).unwrap().chunk_shape();
    assert_eq!(steps(1), [72, 4, 40, 40]);
    assert_eq!(steps(3), [24, 4, 40, 40]);
    assert_eq!(steps(6), [12, 4, 40, 40]);
    assert_eq!(steps(24), [3, 4, 40, 40]);

    // 20°, 70°, 60°, 30° of latitude and 80°, 40°, 80°, 100°, 60° of longitude.
    let tenth = Layout::new(100_000, 1, 1).unwrap();
    assert_eq!(tenth.shape, [1, 4, 1800, 3600]);
    assert_eq!(tenth.lat_shards, [200, 700, 600, 300]);
    assert_eq!(tenth.lon_shards, [800, 400, 800, 1000, 600]);
    assert_eq!(tenth.chunk_shape(), [72, 4, 100, 100]);
    let one = Layout::new(1_000_000, 1, 1).unwrap();
    assert_eq!(one.shape, [1, 4, 180, 360]);
    assert_eq!(one.lat_shards, [20, 70, 60, 30]);
    assert_eq!(one.lon_shards, [80, 40, 80, 100, 60]);
    for layout in [tenth, one] {
        assert_eq!(layout.lat_shards.iter().sum::<u64>(), layout.shape[2]);
        assert_eq!(layout.lon_shards.iter().sum::<u64>(), layout.shape[3]);
        assert_eq!((layout.latitude(0), layout.longitude(0)), (90.0, -180.0));
    }
    assert_eq!(tenth.latitude(1799), -89.9);
    assert_eq!(tenth.longitude(3599), 179.9);

    assert!(
        Layout::new(300_000, 1, 1).is_err(),
        "0.3° does not tile 10°"
    );
}

/// The shards are written here by hand, so they are read by something else:
/// `zarrs` resolves the rectilinear grid, finds each inner chunk through the
/// shard index, checks its CRC-32C and decodes it.
#[test]
fn zarrs_reads_back_what_was_written() {
    use zarrs::array::{Array, ArraySubset};
    use zarrs::filesystem::FilesystemStore;

    // One degree, six-hourly: twelve steps to a chunk, so fourteen steps are
    // a full time chunk and a partial one.
    let layout = Layout::new(1_000_000, 14, 6).expect("layout");
    let out = tempfile::tempdir().expect("tempdir");
    let mut writer =
        Writer::create(out.path(), layout, "t", "s", "2026-09-02T00:00:00").expect("create");

    // A value that says where it belongs, so a chunk in the wrong place, or
    // one with its rows and columns exchanged, cannot pass. An integer under
    // 2048, which Float16 holds exactly.
    let value_at = |t: u64, p: u64, row: u64, column: u64| -> f16 {
        f16::from_f32(((t * 4 + p) * 32 + (row * 7 + column * 3) % 32) as f32)
    };
    // Tile rows 0-1 are the arctic band and 2-8 the northern; tile columns
    // 8-11 the americas and 12-19 the atlantic. (2, 12) is the first inner
    // chunk of the North Atlantic shard and (8, 19) its last.
    let written = [(1, 11), (2, 12), (8, 19), (2, 11)];
    for time_index in 0..2u64 {
        let steps = if time_index == 0 { 12 } else { 2 };
        for &(tile_row, tile_column) in &written {
            let mut values = vec![f16::NAN; layout.chunk_len()];
            for t in 0..steps {
                for p in 0..4 {
                    for r in 0..10 {
                        for c in 0..10 {
                            values[(((t * 4 + p) * 10 + r) * 10 + c) as usize] = value_at(
                                time_index * 12 + t,
                                p,
                                tile_row * 10 + r,
                                tile_column * 10 + c,
                            );
                        }
                    }
                }
            }
            assert!(
                writer
                    .write_chunk(time_index, tile_row, tile_column, &values)
                    .expect("write")
            );
        }
        // Nothing but the fill value is not stored.
        let empty = vec![f16::NAN; layout.chunk_len()];
        assert!(
            !writer
                .write_chunk(time_index, 3, 13, &empty)
                .expect("write")
        );
        assert!(
            !writer
                .write_chunk(time_index, 17, 0, &empty)
                .expect("write")
        );
        writer.finish_time().expect("finish");
    }

    // Three shards hold something in each time chunk, and only those exist.
    for time_index in 0..2 {
        let mut shards: Vec<String> = Vec::new();
        for lat in std::fs::read_dir(out.path().join(format!("data/c/{time_index}/0"))).unwrap() {
            let lat = lat.unwrap();
            for lon in std::fs::read_dir(lat.path()).unwrap() {
                shards.push(format!(
                    "{}/{}",
                    lat.file_name().to_string_lossy(),
                    lon.unwrap().file_name().to_string_lossy()
                ));
            }
        }
        shards.sort();
        assert_eq!(shards, ["0/1", "1/1", "1/2"]);
    }

    let store = Arc::new(FilesystemStore::new(out.path()).expect("store"));
    let array = Array::open(store.clone(), "/data").expect("open");
    assert_eq!(array.shape(), &[14, 4, 180, 360]);
    assert_eq!(
        array.dimension_names().as_ref().map(|names| names
            .iter()
            .map(|name| name.as_deref().unwrap_or("").to_owned())
            .collect::<Vec<_>>()),
        Some(vec![
            "time".to_owned(),
            "param".to_owned(),
            "latitude".to_owned(),
            "longitude".to_owned()
        ])
    );

    // A window over the corner where four shards meet, through both time
    // chunks: rows 10-29 and columns 110-129 are three written tiles and one,
    // in the arctic Atlantic shard that was never made, that is not.
    let subset = ArraySubset::new_with_ranges(&[0..14, 0..4, 10..30, 110..130]);
    let window: Vec<f16> = array.retrieve_array_subset(&subset).expect("read");
    let mut index = 0;
    for t in 0..14 {
        for p in 0..4 {
            for row in 10..30 {
                for column in 110..130 {
                    let got = window[index];
                    index += 1;
                    let tile = (row / 10, column / 10);
                    if written.contains(&tile) {
                        assert_eq!(got, value_at(t, p, row, column), "[{t},{p},{row},{column}]");
                    } else {
                        assert!(got.is_nan(), "[{t},{p},{row},{column}] is {got}");
                    }
                }
            }
        }
    }
    // The far corner of the North Atlantic shard, and a chunk left out of it.
    let corner = ArraySubset::new_with_ranges(&[13..14, 3..4, 89..90, 199..200]);
    let got: Vec<f16> = array.retrieve_array_subset(&corner).expect("read");
    assert_eq!(got, [value_at(13, 3, 89, 199)]);
    let omitted = ArraySubset::new_with_ranges(&[0..1, 0..1, 30..40, 130..140]);
    let got: Vec<f16> = array.retrieve_array_subset(&omitted).expect("read");
    assert!(got.iter().all(|value| value.is_nan()));

    // And the clock: six-hourly steps counted in hours.
    let time = Array::open(store, "/time").expect("open time");
    let hours: Vec<i64> = time
        .retrieve_array_subset(&ArraySubset::new_with_ranges(&[0..14]))
        .expect("read time");
    assert_eq!(hours, (0..14).map(|t| t * 6).collect::<Vec<i64>>());
    assert_eq!(
        time.attributes()["units"],
        "hours since 2026-09-02T00:00:00"
    );
}

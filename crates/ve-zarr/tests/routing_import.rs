#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::path::Path;
use std::sync::Arc;

use ve_zarr::export::{Layout, Writer, f16};
use ve_zarr::routing::RoutingStore;
use zarrs::array::{ArrayBuilder, data_type};
use zarrs::filesystem::FilesystemStore;

#[test]
fn reads_the_original_python_coordinate_chunks() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/routing_test");
    let store = RoutingStore::open(&path).unwrap();
    assert_eq!(store.parameters, ["u10", "v10", "ucur", "vcur"]);
    assert_eq!(store.times.len(), 744);
    assert_eq!(store.times[0], 1_325_376_000);
    assert_eq!(store.times[743] - store.times[0], 743 * 3600);
    assert_eq!((store.latitude[0], store.latitude[719]), (90.0, -89.75));
    assert_eq!(
        (store.longitude[0], store.longitude[1439]),
        (-180.0, 179.75)
    );
    // The fixture deliberately has no data shards: absent data is the fill,
    // never zero wind and never an error opening a sparse export.
    assert!(store.read(0..1).unwrap().iter().all(|v| v.is_nan()));
    assert!(store.read(744..745).is_err());
}

#[test]
fn float32_rounding_does_not_reject_a_tenth_degree_export() {
    let out = tempfile::tempdir().unwrap();
    Writer::create(
        out.path(),
        Layout::new(100_000, 1, 1).unwrap(),
        "",
        "",
        "2012-01-01T00:00:00",
    )
    .unwrap();
    let store = RoutingStore::open(out.path()).unwrap();
    assert_eq!(store.longitude.len(), 3600);
    assert_eq!(store.latitude.len(), 1800);
}

#[test]
fn reads_shards_partial_time_chunks_and_missing_chunks() {
    let out = tempfile::tempdir().unwrap();
    let layout = Layout::new(1_000_000, 14, 6).unwrap();
    let mut writer = Writer::create(out.path(), layout, "t", "s", "2012-01-01T00:00:00").unwrap();
    for time in 0..2 {
        for (row, col) in [(1, 11), (2, 12), (8, 19)] {
            let mut values = vec![f16::NAN; layout.chunk_len()];
            for t in 0..12 {
                for p in 0..4 {
                    for r in 0..10 {
                        for c in 0..10 {
                            values[((t * 4 + p) * 10 + r) * 10 + c] =
                                f16::from_f32((time * 120 + t * 4 + p + row + col) as f32);
                        }
                    }
                }
            }
            writer
                .write_chunk(time as u64, row as u64, col as u64, &values)
                .unwrap();
        }
        writer.finish_time().unwrap();
    }
    let store = RoutingStore::open(out.path()).unwrap();
    let got = store.read(11..14).unwrap();
    let at =
        |t: usize, p: usize, row: usize, col: usize| got[((t * 4 + p) * 180 + row) * 360 + col];
    assert_eq!(at(0, 0, 10, 110), 56.0);
    assert_eq!(at(1, 2, 20, 120), 136.0);
    assert_eq!(at(2, 3, 89, 199), 154.0);
    assert!(at(0, 0, 30, 130).is_nan(), "omitted inner chunk");
    assert!(at(0, 0, 170, 0).is_nan(), "omitted shard");
}

/// Written independently through zarrs: float32, a regional grid, shuffled
/// parameter order, and noncontiguous time coordinates.
fn regional(path: &Path, parameters: &[&str], latitudes: &[f32]) {
    let store = Arc::new(FilesystemStore::new(path).unwrap());
    for (name, values) in [
        ("/longitude", &[-20.0, -19.0, -18.0][..]),
        ("/latitude", latitudes),
    ] {
        let array = ArrayBuilder::new(vec![3], vec![3], data_type::float32(), 0.0f32)
            .build(store.clone(), name)
            .unwrap();
        array.store_metadata().unwrap();
        array.store_chunk(&[0], values.to_vec()).unwrap();
    }
    let time = ArrayBuilder::new(vec![2], vec![2], data_type::int64(), 0i64)
        .attributes(
            serde_json::from_value(serde_json::json!({
                "units": "hours since 2012-01-01T00:00:00", "calendar": "proleptic_gregorian"
            }))
            .unwrap(),
        )
        .build(store.clone(), "/time")
        .unwrap();
    time.store_metadata().unwrap();
    time.store_chunk(&[0], vec![6i64, 12]).unwrap();
    let param = ArrayBuilder::new(vec![4], vec![4], data_type::string(), "")
        .build(store.clone(), "/param")
        .unwrap();
    param.store_metadata().unwrap();
    param.store_chunk(&[0], parameters.to_vec()).unwrap();
    let data = ArrayBuilder::new(
        vec![2, 4, 3, 3],
        vec![2, 4, 3, 3],
        data_type::float32(),
        f32::NAN,
    )
    .dimension_names(Some(["time", "param", "latitude", "longitude"]))
    .attributes(serde_json::from_value(serde_json::json!({"units": "m s-1"})).unwrap())
    .build(store, "/data")
    .unwrap();
    data.store_metadata().unwrap();
    data.store_chunk(&[0, 0, 0, 0], (0..72).map(|v| v as f32).collect::<Vec<_>>())
        .unwrap();
}

#[test]
fn coordinates_and_parameter_names_are_read_instead_of_assumed() {
    let out = tempfile::tempdir().unwrap();
    regional(
        out.path(),
        &["vcur", "u10", "ucur", "v10"],
        &[40.0, 39.0, 38.0],
    );
    let store = RoutingStore::open(out.path()).unwrap();
    assert_eq!(store.parameters, ["vcur", "u10", "ucur", "v10"]);
    assert_eq!(store.times, [1_325_397_600, 1_325_419_200]);
    assert_eq!(store.latitude, [40.0, 39.0, 38.0]);
    assert_eq!(store.longitude, [-20.0, -19.0, -18.0]);
    assert_eq!(
        store.read(1..2).unwrap(),
        (36..72).map(|v| v as f32).collect::<Vec<_>>()
    );
}

#[test]
fn rejects_incomplete_vectors_and_irregular_axes() {
    let out = tempfile::tempdir().unwrap();
    regional(
        out.path(),
        &["u10", "v10", "ucur", "other"],
        &[40.0, 39.0, 38.0],
    );
    assert!(
        RoutingStore::open(out.path())
            .unwrap_err()
            .to_string()
            .contains("both ucur and vcur")
    );
    regional(
        out.path(),
        &["u10", "v10", "ucur", "vcur"],
        &[40.0, 38.5, 38.0],
    );
    assert!(
        RoutingStore::open(out.path())
            .unwrap_err()
            .to_string()
            .contains("regularly spaced")
    );
}

#[test]
#[ignore = "reads the external store named by VE_ROUTING_STORE"]
fn reads_a_local_routing_store() {
    let path = std::env::var("VE_ROUTING_STORE").expect("set VE_ROUTING_STORE");
    let store = RoutingStore::open(Path::new(&path)).unwrap();
    let values = store.read(0..1).unwrap();
    assert_eq!(
        values.len(),
        store.parameters.len() * store.latitude.len() * store.longitude.len()
    );
    assert!(values.iter().any(|v| v.is_finite()));
    assert!(values.iter().any(|v| v.is_nan()));
    println!(
        "{store:?}: {} finite samples in first frame",
        values.iter().filter(|v| v.is_finite()).count()
    );
}

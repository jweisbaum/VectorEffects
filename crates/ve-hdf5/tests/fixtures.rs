//! The reader against files the reference HDF5 library wrote
//! (`fixtures/make.py`). Every expected value is the formula the file was
//! written from, so nothing here trusts the reader to check itself.
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "helpers in an integration test file, outside clippy's test allowance"
)]

use std::path::PathBuf;

use ve_hdf5::{Error, File};

fn fixture(name: &str) -> File {
    let path: PathBuf = [env!("CARGO_MANIFEST_DIR"), "tests", "fixtures", name]
        .iter()
        .collect();
    File::open(&path).expect("fixture opens")
}

/// What both layouts hold: the netCDF-4 shape (superblock 2) and CMC's
/// (superblock 0, symbol-table groups, version 1 object headers).
const LAYOUTS: [&str; 2] = ["netcdf4.h5", "superblock0.h5"];

#[test]
fn the_root_group_lists_every_dataset() {
    for name in LAYOUTS {
        let file = fixture(name);
        let mut names = file.names().unwrap();
        names.sort();
        assert_eq!(
            names,
            ["attributes", "big", "many", "sst", "time", "wind"],
            "{name}"
        );
    }
}

#[test]
fn a_chunked_deflated_shuffled_float_reads_whole() {
    for name in LAYOUTS {
        let file = fixture(name);
        let wind = file.dataset("wind").unwrap();
        assert_eq!(wind.shape(), [3, 5, 7], "{name}");
        let values = wind.read_f32().unwrap();
        assert_eq!(values.len(), 3 * 5 * 7);
        for t in 0..3 {
            for j in 0..5 {
                for i in 0..7 {
                    let expected = if (t, j, i) == (1, 2, 3) {
                        -9999.0
                    } else {
                        (t * 100 + j * 10 + i) as f32 + 0.5
                    };
                    assert_eq!(values[(t * 5 + j) * 7 + i], expected, "{name} {t} {j} {i}");
                }
            }
        }
        assert_eq!(wind.attribute("units").unwrap().text(), Some("m s-1"));
        assert_eq!(
            wind.attribute("_FillValue").unwrap().number(),
            Some(-9999.0)
        );
    }
}

#[test]
fn packed_shorts_read_with_their_packing_attributes() {
    for name in LAYOUTS {
        let file = fixture(name);
        let sst = file.dataset("sst").unwrap();
        assert_eq!(sst.shape(), [4, 6]);
        let values = sst.read_f64().unwrap();
        for (k, value) in values.iter().enumerate() {
            let expected = if k == 23 {
                -32768.0
            } else {
                k as f64 * 50.0 - 300.0
            };
            assert_eq!(*value, expected, "{name} {k}");
        }
        let number = |a: &str| sst.attribute(a).and_then(|a| a.number());
        assert_eq!(number("scale_factor").map(|v| v as f32), Some(0.01));
        assert_eq!(number("add_offset").map(|v| v as f32), Some(273.15));
        assert_eq!(number("_FillValue"), Some(-32768.0));
        assert_eq!(
            sst.attribute("long_name").and_then(|a| a.text()),
            Some("sea surface foundation temperature"),
            "{name}: a variable-length string from the global heap"
        );
        assert_eq!(
            sst.attribute("units").and_then(|a| a.text()),
            Some("kelvin")
        );
        for k in 0..10 {
            assert_eq!(
                number(&format!("extra_{k}")),
                Some(k as f64 * 3.0),
                "{name}"
            );
        }
    }
}

#[test]
fn contiguous_big_endian_and_many_chunk_datasets_read() {
    for name in LAYOUTS {
        let file = fixture(name);
        let time = file.dataset("time").unwrap();
        assert_eq!(time.read_f64().unwrap(), [420768.0, 420774.0]);
        assert_eq!(
            time.attribute("units").and_then(|a| a.text()),
            Some("hours since 1978-01-01")
        );
        assert_eq!(
            file.dataset("big").unwrap().read_f64().unwrap(),
            [1.5, -2.25, 1e10]
        );
        let many = file.dataset("many").unwrap().read_f64().unwrap();
        let expected: Vec<f64> = (0..200).map(|k| (k % 120) as f64).collect();
        assert_eq!(
            many, expected,
            "{name}: a hundred chunks, past one B-tree node"
        );
    }
}

#[test]
fn two_hundred_attributes_all_read() {
    // Dense storage in the netCDF-4 layout, with an indirect heap block and
    // an internal name-index node; compact, behind continuations, in the old.
    for name in LAYOUTS {
        let file = fixture(name);
        let dataset = file.dataset("attributes").unwrap();
        assert_eq!(dataset.attributes().len(), 200, "{name}");
        for k in 0..200 {
            let attribute = dataset.attribute(&format!("a{k:03}")).unwrap();
            let expected: Vec<f64> = (0..8).map(|i| (i + k) as f64).collect();
            assert_eq!(
                attribute.numbers(),
                Some(expected.as_slice()),
                "{name} a{k:03}"
            );
        }
    }
}

#[test]
fn global_attributes_are_the_root_groups() {
    for name in LAYOUTS {
        let file = fixture(name);
        let attributes = file.attributes().unwrap();
        let find = |n: &str| attributes.iter().find(|a| a.name() == n);
        for k in 0..12 {
            assert_eq!(
                find(&format!("global_{k}")).and_then(|a| a.number()),
                Some(k as f64 / 4.0)
            );
        }
        assert_eq!(
            find("title").and_then(|a| a.text()),
            Some("ve-hdf5 fixture")
        );
    }
}

#[test]
fn links_in_dense_storage_are_found() {
    let file = fixture("dense_links.h5");
    let mut names = file.names().unwrap();
    names.sort();
    let expected: Vec<String> = (0..20).map(|k| format!("v{k:02}")).collect();
    assert_eq!(names, expected);
    for k in 0..20 {
        let values = file
            .dataset(&format!("v{k:02}"))
            .unwrap()
            .read_f64()
            .unwrap();
        assert_eq!(values, [k as f64; 3]);
    }
}

#[test]
fn a_missing_dataset_is_named() {
    let file = fixture("netcdf4.h5");
    match file.dataset("u_wind") {
        Err(Error::NotFound(name)) => assert_eq!(name, "u_wind"),
        other => panic!("expected NotFound, got {other:?}"),
    }
}

#[test]
fn something_else_is_not_hdf5() {
    assert!(matches!(
        File::from_bytes(b"CDF\x01 not an hdf5 file at all".to_vec()),
        Err(Error::NotHdf5)
    ));
}

/// Damage anywhere in the file is an error, never a panic, a hang or an
/// allocation the size of a corrupt length.
#[test]
fn damage_is_an_error_not_a_panic() {
    let path: PathBuf = [env!("CARGO_MANIFEST_DIR"), "tests", "fixtures"]
        .iter()
        .collect();
    for name in ["netcdf4.h5", "superblock0.h5", "dense_links.h5"] {
        let bytes = std::fs::read(path.join(name)).unwrap();
        for at in (0..bytes.len()).step_by(37) {
            for flip in [0xFF, 0x80, 0x01] {
                let mut damaged = bytes.clone();
                damaged[at] ^= flip;
                read_everything(damaged);
            }
        }
        for cut in (0..bytes.len()).step_by(997) {
            read_everything(bytes[..cut].to_vec());
        }
    }
}

fn read_everything(bytes: Vec<u8>) {
    let Ok(file) = File::from_bytes(bytes) else {
        return;
    };
    let _ = file.attributes();
    let Ok(names) = file.names() else { return };
    for name in names {
        if let Ok(dataset) = file.dataset(&name) {
            let _ = dataset.read_f32();
        }
    }
}

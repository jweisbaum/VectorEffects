//! The reader against the files it exists for, checked against values the
//! reference library (h5py over HDF5 1.10) read from the same files. The
//! files are 44 MB and not committed:
//!
//! ```bash
//! VE_TEST_HDF5_DIR=<dir with nbs.nc, ostia.nc, cmc.nc> \
//!     cargo test -p ve-hdf5 --release --test real_files -- --ignored --nocapture
//! ```
//!
//! `nbs.nc` is NOAA's Blended Seawinds NRT for 2026-10-01
//! (`NBSv02_wind_6hourly_20261001_nrt.nc`), `ostia.nc` the Met Office's
//! OSTIA for 2015-09-01, `cmc.nc` CMC 0.1° for 2011-04-30.
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "helpers in an integration test file, outside clippy's test allowance"
)]

use std::path::PathBuf;
use std::time::Instant;

use ve_hdf5::File;

fn open(name: &str) -> File {
    let dir =
        std::env::var("VE_TEST_HDF5_DIR").expect("VE_TEST_HDF5_DIR names the sample directory");
    let started = Instant::now();
    let file = File::open(&PathBuf::from(dir).join(name)).unwrap();
    println!("{name}: opened in {:?}", started.elapsed());
    file
}

fn read(file: &File, name: &str, shape: &[u64]) -> Vec<f64> {
    let dataset = file.dataset(name).unwrap();
    assert_eq!(dataset.shape(), shape, "{name}");
    let started = Instant::now();
    let values = dataset.read_f64().unwrap();
    println!(
        "  {name}: {} values in {:?}",
        values.len(),
        started.elapsed()
    );
    values
}

fn at(values: &[f64], expected: &[(usize, f64)]) {
    for &(k, v) in expected {
        let got = values[k];
        assert!(
            got == v || (got.is_nan() && v.is_nan()),
            "at {k}: {got} where h5py read {v}"
        );
    }
}

#[test]
#[ignore = "reads $VE_TEST_HDF5_DIR"]
fn blended_seawinds() {
    let file = open("nbs.nc");
    let mut names = file.names().unwrap();
    names.sort();
    assert_eq!(
        names,
        [
            "crs", "lat", "lon", "mask", "time", "u_wind", "v_wind", "zlev"
        ]
    );
    for (name, sum) in [
        ("u_wind", 2_100_960.073_949_365),
        ("v_wind", 1_244_349.336_617_840_2),
    ] {
        let values = read(&file, name, &[4, 1, 719, 1440]);
        let finite: Vec<f64> = values.iter().copied().filter(|v| v.is_finite()).collect();
        assert_eq!(finite.len(), 2_823_297, "{name}");
        let total: f64 = finite.iter().sum();
        assert!((total - sum).abs() < 1e-3, "{name}: {total} against {sum}");
    }
    let u = read(&file, "u_wind", &[4, 1, 719, 1440]);
    at(
        &u,
        &[
            (0, f64::NAN),
            (591_634, -6.40876579284668),
            (1_380_480, 9.253840446472168),
            (4_141_439, 8.71004867553711),
        ],
    );
    let lat = read(&file, "lat", &[719]);
    at(&lat, &[(0, -89.75), (359, 0.0), (718, 89.75)]);
    let lon = read(&file, "lon", &[1440]);
    at(&lon, &[(0, 0.0), (1439, 359.75)]);
    assert_eq!(
        read(&file, "time", &[4]),
        [427_320.0, 427_326.0, 427_332.0, 427_338.0]
    );
    let mask: f64 = read(&file, "mask", &[4, 1, 719, 1440]).iter().sum();
    assert_eq!(mask, 7_799_433.0);
    let wind = file.dataset("u_wind").unwrap();
    assert_eq!(
        wind.attribute("units").and_then(|a| a.text()),
        Some("m s-1")
    );
    assert_eq!(
        wind.attribute("_FillValue").and_then(|a| a.number()),
        Some(-9999.0)
    );
    let title = file.attributes().unwrap();
    let title = title
        .iter()
        .find(|a| a.name() == "title")
        .and_then(|a| a.text());
    assert_eq!(
        title,
        Some("NOAA/NCEI Blended 6-hourly 0.25-degree Sea Surface Wind Version 2.0")
    );
}

#[test]
#[ignore = "reads $VE_TEST_HDF5_DIR"]
fn ostia() {
    let file = open("ostia.nc");
    let sst = read(&file, "analysed_sst", &[1, 3600, 7200]);
    assert_eq!(sst.iter().sum::<f64>(), -259_196_453_436.0);
    at(
        &sst,
        &[
            (0, -32768.0),
            (3_702_857, -177.0),
            (8_640_000, 1837.0),
            (12_960_000, 2970.0),
            (25_919_999, -180.0),
        ],
    );
    let lat = read(&file, "lat", &[3600]);
    at(
        &lat,
        &[(0, f64::from(-89.975_f32)), (3599, f64::from(89.975_f32))],
    );
    assert_eq!(read(&file, "time", &[1]), [1_441_108_800.0]);
    let dataset = file.dataset("analysed_sst").unwrap();
    let number = |a: &str| dataset.attribute(a).and_then(|a| a.number());
    assert_eq!(number("scale_factor").map(|v| v as f32), Some(0.01));
    assert_eq!(number("add_offset").map(|v| v as f32), Some(273.15));
    assert_eq!(number("_FillValue"), Some(-32768.0));
    assert_eq!(
        dataset.attribute("units").and_then(|a| a.text()),
        Some("kelvin")
    );
}

#[test]
#[ignore = "reads $VE_TEST_HDF5_DIR"]
fn cmc() {
    let file = open("cmc.nc");
    let sst = read(&file, "analysed_sst", &[1, 1801, 3600]);
    assert_eq!(sst.iter().sum::<f64>(), 7_830_650_821.0);
    at(
        &sst,
        &[
            (0, -180.0),
            (926_228, 57.0),
            (2_161_200, 2222.0),
            (3_241_800, 2889.0),
            (6_483_599, -180.0),
        ],
    );
    let lon = read(&file, "lon", &[3600]);
    at(
        &lon,
        &[(0, -180.0), (1800, 0.0), (3599, f64::from(179.9_f32))],
    );
    assert_eq!(read(&file, "time", &[1]), [1_304_078_400.0]);
    let mask: f64 = read(&file, "mask", &[1, 1801, 3600]).iter().sum();
    assert_eq!(mask, 14_583_301.0);
    let title = file.attributes().unwrap();
    let title = title
        .iter()
        .find(|a| a.name() == "title")
        .and_then(|a| a.text());
    assert_eq!(
        title,
        Some("CMC 0.1 deg global sea surface temperature analysis")
    );
}

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "test code; clippy's allow-in-tests does not reach tests/"
)]
//! The import decoder against the writer's own output.
//!
//! Every field the app can export must import back as the same field, with
//! its components on the right axes, its rows in the right order and its
//! time steps in the right sequence. The values are an analytic function of
//! position so a transposed axis or a swapped component shows up as a number,
//! not a plausible-looking picture.

use ve_core::project::FieldKind;
use ve_grib::decode;
use ve_grib::import;
use ve_grib::writer::{GridSpec, MessageSpec, Parameter, ReferenceTime, message};

fn reference_time() -> ReferenceTime {
    ReferenceTime {
        year: 2026,
        month: 9,
        day: 3,
        hour: 0,
        minute: 0,
        second: 0,
    }
}

fn grid() -> GridSpec {
    GridSpec {
        ni: 360,
        nj: 181,
        micro_degrees: 1_000_000,
    }
}

/// `u` rises with longitude, `v` with latitude, both offset by the hour.
fn field(hour: u32) -> (Vec<f32>, Vec<f32>) {
    let mut u = Vec::new();
    let mut v = Vec::new();
    for (lon, lat) in grid().points() {
        u.push((lon / 10.0) as f32 + hour as f32);
        v.push((lat / 10.0) as f32 - hour as f32);
    }
    (u, v)
}

fn file(kind: FieldKind, hours: &[u32]) -> Vec<u8> {
    let (pu, pv) = match kind {
        FieldKind::Wind => (Parameter::WindU, Parameter::WindV),
        FieldKind::Current => (Parameter::CurrentU, Parameter::CurrentV),
    };
    let mut out = Vec::new();
    for &hour in hours {
        let (u, v) = field(hour);
        for (parameter, values) in [(pu, &u), (pv, &v)] {
            let spec = MessageSpec {
                parameter,
                grid: grid(),
                reference_time: reference_time(),
                forecast_hour: hour,
                centre: 255,
                bits: 16,
            };
            out.extend(message(&spec, values).unwrap());
        }
    }
    out
}

#[test]
fn every_message_of_an_export_decodes_with_its_identity() {
    let bytes = file(FieldKind::Wind, &[0, 3]);
    let decoded = decode::read_all(&bytes).unwrap();
    assert!(decoded.skipped.is_empty(), "{:?}", decoded.skipped);
    assert_eq!(decoded.messages.len(), 4);

    let first = &decoded.messages[0].header;
    assert_eq!((first.discipline, first.category, first.number), (0, 2, 2));
    assert_eq!(first.forecast_hours, 0.0);
    assert_eq!((first.surface_type, first.surface_value), (103, 10.0));
    let lattice = first.grid.lat_lon().expect("the writer emits template 3.0");
    assert_eq!((lattice.ni, lattice.nj), (360, 181));
    assert_eq!((lattice.la1, lattice.lo1), (90.0, 0.0));
    assert_eq!((lattice.di, lattice.dj), (1.0, 1.0));
    assert_eq!(lattice.scan, 0);
    assert_eq!(decoded.messages[3].header.forecast_hours, 3.0);
    assert_eq!(decoded.messages[3].header.number, 3);

    // 2026-09-03T03:00Z.
    assert_eq!(decoded.messages[3].header.valid_unix_s(), 1_788_404_400);
}

#[test]
fn an_exported_field_imports_back_on_the_right_axes() {
    let bytes = file(FieldKind::Wind, &[0]);
    let decoded = decode::read_all(&bytes).unwrap();
    let sequences = import::sequences(decoded.messages, None).unwrap();
    assert_eq!(sequences.len(), 1);
    let grid = &sequences[0].frames[0].grid;
    assert_eq!(sequences[0].kind, FieldKind::Wind);
    assert!(grid.wraps);
    assert_eq!((grid.lon0, grid.lat0), (0.0, 90.0));

    // The writer starts its rows at the prime meridian and runs north to
    // south, so canonical order is the file's order — but the values are
    // what prove it: u follows longitude, v follows latitude.
    let tolerance = 0.01;
    let at = |lon: f64, lat: f64| grid.sample(lon, lat).expect("covered");
    let s = at(30.0, 40.0);
    assert!((f64::from(s.u) - 3.0).abs() < tolerance, "{s:?}");
    assert!((f64::from(s.v) - 4.0).abs() < tolerance, "{s:?}");
    let s = at(-30.0, -40.0);
    assert!((f64::from(s.u) + 3.0).abs() < tolerance, "{s:?}");
    assert!((f64::from(s.v) + 4.0).abs() < tolerance, "{s:?}");
    let s = at(179.0, 89.0);
    assert!((f64::from(s.u) - 17.9).abs() < tolerance, "{s:?}");
    assert!((f64::from(s.v) - 8.9).abs() < tolerance, "{s:?}");
}

#[test]
fn time_steps_come_back_in_order_with_relative_offsets() {
    // Written out of order on purpose.
    let bytes = file(FieldKind::Current, &[6, 0, 3]);
    let imported = import::sequences(decode::read_all(&bytes).unwrap().messages, None).unwrap();
    let sequence = &imported[0];
    assert_eq!(sequence.kind, FieldKind::Current);
    let offsets: Vec<f64> = sequence.frames.iter().map(|f| f.offset_hours).collect();
    assert_eq!(offsets, vec![0.0, 3.0, 6.0]);
    // The 6 h frame carries the 6 h field: u offset by 6.
    let s = sequence.frames[2].grid.sample(0.0, 0.0).unwrap();
    assert!((f64::from(s.u) - 6.0).abs() < 0.01, "{s:?}");
    assert_eq!(sequence.span_hours(), 6.0);
}

#[test]
fn a_file_with_wind_and_currents_imports_as_two_sequences() {
    let mut bytes = file(FieldKind::Current, &[0, 3]);
    bytes.extend(file(FieldKind::Wind, &[0, 3]));
    let imported = import::sequences(decode::read_all(&bytes).unwrap().messages, None).unwrap();
    assert_eq!(imported.len(), 2);
    assert_eq!(
        imported[0].kind,
        FieldKind::Wind,
        "wind first, whatever the file order"
    );
    assert_eq!(imported[1].kind, FieldKind::Current);
}

#[test]
fn reading_a_file_from_disk_selects_only_vector_components() {
    let dir = std::env::temp_dir().join(format!("ve-grib-import-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("wind.grib2");
    std::fs::write(&path, file(FieldKind::Wind, &[0, 1, 2])).unwrap();
    let imported = import::read_file(&path, None).unwrap();
    assert_eq!(imported.sequences.len(), 1);
    assert_eq!(imported.sequences[0].frames.len(), 3);
    assert!(imported.skipped.is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn junk_between_messages_is_skipped_and_junk_alone_is_refused() {
    let mut bytes = b"\n\n".to_vec();
    bytes.extend(file(FieldKind::Wind, &[0]));
    bytes.extend_from_slice(b"\r\n");
    assert_eq!(decode::read_all(&bytes).unwrap().messages.len(), 2);
    assert!(decode::read_all(b"not a grib at all").is_err());
}

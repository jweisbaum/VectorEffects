#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "test code; clippy's allow-in-tests does not reach tests/"
)]
//! Every packing the forecast centres ship, against one known field.
//!
//! The five fixtures carry the *same* analytic field in five packings (see
//! `fixtures/README.md`), so each decoder is held to an answer that exists
//! independently of it: `u = lon/10 + hour` and `v = lat/10 - hour` on a 10°
//! grid, written by the app's own writer and repacked by ecCodes. A
//! transposed axis, a swapped component or a misread sample width is a
//! number here rather than a plausible-looking picture.

use ve_grib::decode::{self, MISSING};

const SIMPLE: &[u8] = include_bytes!("fixtures/simple.grib2");
const CCSDS: &[u8] = include_bytes!("fixtures/ccsds.grib2");
const JPEG2000: &[u8] = include_bytes!("fixtures/jpeg2000.grib2");
const COMPLEX: &[u8] = include_bytes!("fixtures/complex.grib2");
const COMPLEX_SD: &[u8] = include_bytes!("fixtures/complex_sd.grib2");
const CCSDS_BITMAP: &[u8] = include_bytes!("fixtures/ccsds_bitmap.grib2");
const JPEG2000_BITMAP: &[u8] = include_bytes!("fixtures/jpeg2000_bitmap.grib2");

const NI: usize = 36;
const NJ: usize = 19;
const POINTS: usize = NI * NJ;

/// The nodes `fixtures/README.md` says the bitmapped files leave out.
const MISSING_NODES: &[usize] = &[0];
const MISSING_RUN: std::ops::Range<usize> = 200..260;

/// The field the fixtures were built from, at message `index`.
///
/// Messages run u then v at hour 0, then u then v at hour 3. Longitudes are
/// `GridSpec::points`': `0, 10 … 170, -180 … -10`.
fn expected(index: usize) -> Vec<f32> {
    let hour = if index < 2 { 0.0 } else { 3.0 };
    let is_u = index.is_multiple_of(2);
    (0..POINTS)
        .map(|k| {
            let lon = {
                let raw = 10.0 * (k % NI) as f64;
                if raw >= 180.0 { raw - 360.0 } else { raw }
            };
            let lat = 90.0 - 10.0 * (k / NI) as f64;
            if is_u {
                (lon / 10.0) as f32 + hour
            } else {
                (lat / 10.0) as f32 - hour
            }
        })
        .collect()
}

/// Half a packing step at 16 bits over this field's range: the most a value
/// can move by being packed and unpacked, and nothing to do with the codec.
fn tolerance(values: &[f32]) -> f32 {
    let max = values.iter().cloned().fold(f32::MIN, f32::max);
    let min = values.iter().cloned().fold(f32::MAX, f32::min);
    ((max - min) / f32::from(u16::MAX)).max(1e-6)
}

fn check(label: &str, bytes: &[u8], bitmapped: bool) {
    let decoded = decode::read_all(bytes).expect("the fixture decodes");
    assert!(
        decoded.skipped.is_empty(),
        "{label}: {:?} was skipped",
        decoded.skipped
    );
    assert_eq!(decoded.messages.len(), 4, "{label}: four messages");

    for (index, message) in decoded.messages.iter().enumerate() {
        let want = expected(index);
        assert_eq!(
            message.values.len(),
            POINTS,
            "{label} message {index}: value count"
        );
        let tol = tolerance(&want);

        for (k, (&got, &wanted)) in message.values.iter().zip(&want).enumerate() {
            let absent = bitmapped && (MISSING_RUN.contains(&k) || MISSING_NODES.contains(&k));
            if absent {
                assert_eq!(
                    got, MISSING,
                    "{label} message {index} node {k}: the bitmap leaves it out"
                );
            } else {
                assert!(
                    (got - wanted).abs() <= tol,
                    "{label} message {index} node {k}: {got} against {wanted}, \
                     outside the {tol} packing step"
                );
            }
        }
    }
}

#[test]
fn simple_packing_decodes_the_field() {
    check("simple", SIMPLE, false);
}

#[test]
fn complex_packing_decodes_the_field() {
    check("complex", COMPLEX, false);
}

#[test]
fn complex_packing_with_spatial_differencing_decodes_the_field() {
    check("complex+sd", COMPLEX_SD, false);
}

#[test]
fn ccsds_packing_decodes_the_field() {
    check("ccsds", CCSDS, false);
}

#[test]
fn jpeg_2000_packing_decodes_the_field() {
    check("jpeg2000", JPEG2000, false);
}

#[test]
fn ccsds_packing_honours_a_bitmap() {
    check("ccsds+bitmap", CCSDS_BITMAP, true);
}

#[test]
fn jpeg_2000_packing_honours_a_bitmap() {
    check("jpeg2000+bitmap", JPEG2000_BITMAP, true);
}

/// The point of the fixture set: five packings of one field must not merely
/// each be plausible, they must agree with each other. A packing that read
/// its samples a byte or a bit out would still land near the field's range
/// and pass a loose per-value check; it cannot also match the other four
/// value for value.
#[test]
fn every_packing_agrees_with_every_other() {
    let reference = decode::read_all(SIMPLE).expect("simple decodes");
    for (label, bytes) in [
        ("complex", COMPLEX),
        ("complex+sd", COMPLEX_SD),
        ("ccsds", CCSDS),
        ("jpeg2000", JPEG2000),
    ] {
        let other = decode::read_all(bytes).expect("fixture decodes");
        for (index, (a, b)) in reference.messages.iter().zip(&other.messages).enumerate() {
            let tol = tolerance(&a.values);
            for (k, (&x, &y)) in a.values.iter().zip(&b.values).enumerate() {
                assert!(
                    (x - y).abs() <= 2.0 * tol,
                    "{label} message {index} node {k}: {y} against simple's {x}"
                );
            }
        }
    }
}

/// The headers must survive the repacking too: a decoder that read the grid
/// or the forecast hour out of the wrong octets would still produce values.
#[test]
fn the_repacked_files_describe_the_same_grid_and_times() {
    let reference = decode::read_all(SIMPLE).expect("simple decodes");
    for bytes in [
        COMPLEX,
        COMPLEX_SD,
        CCSDS,
        JPEG2000,
        CCSDS_BITMAP,
        JPEG2000_BITMAP,
    ] {
        let other = decode::read_all(bytes).expect("fixture decodes");
        for (a, b) in reference.messages.iter().zip(&other.messages) {
            assert_eq!(a.header.grid, b.header.grid);
            assert_eq!(a.header.forecast_hours, b.header.forecast_hours);
            assert_eq!(a.header.discipline, b.header.discipline);
            assert_eq!(a.header.category, b.header.category);
            assert_eq!(a.header.number, b.header.number);
        }
    }
}

/// A CCSDS stream of signed samples is refused by name rather than read with
/// its sign bit taken for magnitude. Nothing writes one — the shared 5.0
/// formula has no meaning for a negative `X`, its reference being the
/// field's minimum — so the fixture is made by setting the flag.
#[test]
fn signed_ccsds_samples_are_refused_by_name() {
    let mut bytes = CCSDS.to_vec();
    let section5 = find_section(&bytes, 5).expect("the fixture has a section 5");
    // Octet 22 of template 5.42 is the compression options mask; bit 0 is
    // "samples are signed".
    bytes[section5 + 21] |= 0x01;

    // Only the first message is patched, so the other three must still
    // decode: one unreadable field never stops the rest of a file importing.
    let decoded = decode::read_all(&bytes).expect("the file still parses");
    assert_eq!(
        decoded.messages.len(),
        3,
        "the unpatched messages still read"
    );
    assert_eq!(decoded.skipped.len(), 1, "exactly the patched message");
    let reason = &decoded.skipped[0].reason;
    assert!(
        reason.contains("signed"),
        "the reason should name the flag, not describe corruption: {reason}"
    );
}

/// Offset of the first section with the given number in the first message.
fn find_section(bytes: &[u8], number: u8) -> Option<usize> {
    let mut at = 16;
    while at + 5 < bytes.len() {
        let length =
            u32::from_be_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]) as usize;
        if bytes[at + 4] == number {
            return Some(at);
        }
        if length == 0 {
            return None;
        }
        at += length;
    }
    None
}

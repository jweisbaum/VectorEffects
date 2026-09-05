#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "test code; clippy's allow-in-tests does not reach tests/"
)]
//! M19: the packing width is a choice, and every offered width writes a file
//! that reads back within its own step — through the writer's test reader and
//! through the import decoder, which is the one that meets other centres'
//! files (spec.md 12.3).

use ve_grib::packing::{BIT_WIDTHS, nominal_step_mps};
use ve_grib::writer::{GridSpec, MessageSpec, Parameter, ReferenceTime, message};

/// A ten-degree grid: small enough to write four times in a test.
fn grid() -> GridSpec {
    GridSpec {
        ni: 36,
        nj: 19,
        micro_degrees: 10_000_000,
    }
}

fn spec(bits: u8) -> MessageSpec {
    MessageSpec {
        parameter: Parameter::WindU,
        grid: grid(),
        reference_time: ReferenceTime {
            year: 2026,
            month: 9,
            day: 4,
            hour: 0,
            minute: 0,
            second: 0,
        },
        forecast_hour: 0,
        centre: 255,
        bits,
    }
}

/// A field that uses the whole ±60 m/s range, so the step is the nominal one.
fn field() -> Vec<f32> {
    let n = grid().point_count() as usize;
    (0..n)
        .map(|i| -60.0 + 120.0 * (i as f32) / ((n - 1) as f32))
        .collect()
}

#[test]
fn every_width_writes_a_message_that_reads_back_within_its_step() {
    let values = field();
    for bits in BIT_WIDTHS {
        let bytes = message(&spec(bits), &values).expect("writes");
        let step = nominal_step_mps(bits) as f32;

        // The writer's own verifier: the header says the width, the values
        // are within half a step (a little more, for float rounding).
        let decoded = ve_grib::reader::decode(&bytes);
        assert_eq!(decoded.bits, bits, "section 5 records the width");
        for (i, (a, b)) in values.iter().zip(&decoded.values).enumerate() {
            assert!(
                (a - b).abs() <= step * 0.75,
                "{bits} bits, reader, index {i}: {a} vs {b}"
            );
        }

        // The import decoder, which meets every other centre's simple packing
        // and must meet ours at every width too.
        let imported = ve_grib::decode::read_all(&bytes).expect("decodes");
        let first = imported.messages.first().expect("one message");
        for (i, (a, b)) in values.iter().zip(&first.values).enumerate() {
            assert!(
                (a - b).abs() <= step * 0.75,
                "{bits} bits, import, index {i}: {a} vs {b}"
            );
        }
    }
}

/// Narrower is smaller, in the proportion the width says: 8 bits is half of
/// 16, and 24 is half again as much.
#[test]
fn the_file_shrinks_with_the_width() {
    let values = field();
    let size = |bits: u8| message(&spec(bits), &values).expect("writes").len();
    let n = grid().point_count() as usize;
    let data = |bits: u8| (n * usize::from(bits)).div_ceil(8);
    // Same headers, so the difference between two widths is exactly the data.
    assert_eq!(size(16) - size(8), data(16) - data(8));
    assert_eq!(size(24) - size(16), data(24) - data(16));
    assert_eq!(size(16) - size(12), data(16) - data(12));
}

/// Invariant 4 at every width: the same field packs to the same bytes.
#[test]
fn every_width_is_byte_reproducible() {
    let values = field();
    for bits in BIT_WIDTHS {
        let a = message(&spec(bits), &values).expect("writes");
        let b = message(&spec(bits), &values).expect("writes");
        assert_eq!(a, b, "{bits} bits");
    }
}

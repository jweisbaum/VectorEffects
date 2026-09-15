#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "test code; clippy's allow-in-tests does not reach tests/"
)]
//! Decodes what the writer produces and checks it survives the round trip.
//!
//! The decoder lives here and only here. The shipped binary must never depend
//! on an external GRIB library (invariant 5), and a reader in the application
//! would be dead weight — but a reader in the *tests* means every writer change
//! is checked against an independent parse rather than against itself.
//!
//! ecCodes validates the same files in CI. This exists so the round trip is
//! covered even where ecCodes is not installed.

use ve_grib::reader::decode;
use ve_grib::writer::{GridSpec, MessageSpec, Parameter, ReferenceTime, message};

fn reference_time() -> ReferenceTime {
    ReferenceTime {
        year: 2026,
        month: 9,
        day: 2,
        hour: 12,
        minute: 30,
        second: 15,
    }
}

fn spec(grid: GridSpec, parameter: Parameter, hour: u32) -> MessageSpec {
    MessageSpec {
        parameter,
        grid,
        reference_time: reference_time(),
        forecast_hour: hour,
        centre: 255,
        bits: 16,
    }
}

fn grids() -> Vec<GridSpec> {
    vec![
        GridSpec {
            ni: 360,
            nj: 181,
            micro_degrees: 1_000_000,
        },
        GridSpec {
            ni: 720,
            nj: 361,
            micro_degrees: 500_000,
        },
        GridSpec {
            ni: 1440,
            nj: 721,
            micro_degrees: 250_000,
        },
    ]
}

/// M4 acceptance: decoded values match the source within packing tolerance,
/// at every resolution.
#[test]
fn values_survive_the_round_trip_at_every_resolution() {
    for grid in grids() {
        let points: Vec<(f64, f64)> = grid.points().collect();
        let values: Vec<f32> = points
            .iter()
            .map(|(lon, lat)| ((lat / 90.0) * 25.0 + (lon / 180.0) * 5.0) as f32)
            .collect();

        let bytes = message(&spec(grid, Parameter::WindU, 0), &values).expect("builds");
        let decoded = decode(&bytes);

        assert_eq!(decoded.values.len(), values.len(), "{grid:?}");
        let worst = values
            .iter()
            .zip(&decoded.values)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0f32, f32::max);
        assert!(worst < 0.002, "{grid:?}: worst error {worst}");
    }
}

/// The grid header must describe the grid the values are actually on.
#[test]
fn the_grid_header_matches_the_grid() {
    for grid in grids() {
        let values = vec![1.0f32; grid.point_count() as usize];
        let decoded = decode(&message(&spec(grid, Parameter::WindU, 0), &values).expect("builds"));

        assert_eq!((decoded.ni, decoded.nj), (grid.ni, grid.nj));
        assert_eq!(
            (decoded.di, decoded.dj),
            (grid.micro_degrees, grid.micro_degrees)
        );
        assert_eq!(decoded.la1, 90_000_000, "starts at the north pole");
        assert_eq!(decoded.lo1, 0, "starts at the prime meridian");
        assert_eq!(decoded.la2, -90_000_000, "ends at the south pole");
        assert_eq!(
            decoded.lo2,
            360_000_000 - grid.micro_degrees as i32,
            "last column stops one step short of wrapping"
        );
        assert_eq!(decoded.scanning_mode, 0x00, "+i, -j, i consecutive");
        assert_eq!(
            decoded.resolution_flags & 0x08,
            0,
            "bit 5 clear: u/v are earth-relative, not grid-relative"
        );
        assert_eq!(
            decoded.resolution_flags & 0x30,
            0x30,
            "i and j increments given"
        );
    }
}

#[test]
fn the_envelope_carries_the_right_identification() {
    let grid = grids()[0];
    let values = vec![0.0f32; grid.point_count() as usize];
    let decoded = decode(&message(&spec(grid, Parameter::WindU, 24), &values).expect("builds"));

    assert_eq!(decoded.edition, 2);
    assert_eq!(decoded.discipline, 0, "meteorological");
    assert_eq!(decoded.centre, 255);
    assert_eq!(decoded.reference_time, reference_time());
    assert_eq!(decoded.forecast_hour, 24);
}

#[test]
fn each_parameter_decodes_to_its_own_identity() {
    let grid = grids()[0];
    let values = vec![3.0f32; grid.point_count() as usize];

    for (parameter, discipline, category, number, surface, level) in [
        (Parameter::WindU, 0, 2, 2, 103, 10),
        (Parameter::WindV, 0, 2, 3, 103, 10),
        (Parameter::CurrentU, 10, 1, 2, 160, 0),
        (Parameter::CurrentV, 10, 1, 3, 160, 0),
    ] {
        let decoded = decode(&message(&spec(grid, parameter, 0), &values).expect("builds"));
        assert_eq!(decoded.discipline, discipline, "{parameter:?}");
        assert_eq!(decoded.category, category, "{parameter:?}");
        assert_eq!(decoded.number, number, "{parameter:?}");
        assert_eq!(decoded.surface_type, surface, "{parameter:?}");
        assert_eq!(decoded.surface_value, level, "{parameter:?}");
    }
}

/// A calm field is the common case for a new project and must still be a
/// legal, decodable message.
#[test]
fn a_constant_field_decodes() {
    let grid = grids()[0];
    let values = vec![0.0f32; grid.point_count() as usize];
    let decoded = decode(&message(&spec(grid, Parameter::WindV, 0), &values).expect("builds"));
    assert_eq!(decoded.values, values);
}

/// Invariant 4: the same project must export the same bytes, every time.
#[test]
fn output_is_byte_reproducible() {
    let grid = grids()[0];
    let values: Vec<f32> = grid
        .points()
        .map(|(lon, lat)| ((lat * 0.3) + (lon * 0.05)) as f32)
        .collect();

    let spec = spec(grid, Parameter::WindU, 12);
    let first = message(&spec, &values).expect("builds");
    let second = message(&spec, &values).expect("builds");
    assert_eq!(
        first, second,
        "two encodes of the same field must be identical"
    );
}

/// Messages concatenate into one file, each independently decodable.
#[test]
fn concatenated_messages_stay_separable() {
    let grid = grids()[0];
    let values = vec![7.5f32; grid.point_count() as usize];

    let mut file = Vec::new();
    for hour in [0u32, 3, 6] {
        for parameter in [Parameter::WindU, Parameter::WindV] {
            file.extend_from_slice(
                &message(&spec(grid, parameter, hour), &values).expect("builds"),
            );
        }
    }

    // Walk the file the way a decoder would: read a length, skip, repeat.
    let mut at = 0;
    let mut found = Vec::new();
    while at < file.len() {
        assert_eq!(&file[at..at + 4], b"GRIB");
        let length =
            u64::from_be_bytes(file[at + 8..at + 16].try_into().expect("eight bytes")) as usize;
        let decoded = decode(&file[at..at + length]);
        found.push((decoded.forecast_hour, decoded.number));
        at += length;
    }

    assert_eq!(found, vec![(0, 2), (0, 3), (3, 2), (3, 3), (6, 2), (6, 3)]);
}

/// Invariant 4, across machines: a fixed field must encode to fixed bytes.
///
/// The digest is pinned rather than a committed binary fixture — same
/// guarantee, no megabytes in the repository. CI runs this on macOS, Windows
/// and Linux, so a platform-dependent encoding fails here rather than being
/// discovered by a user comparing two exports.
///
/// **If this fails after a deliberate writer change, re-record the digest and
/// say so in the commit.** If it fails without one, something is
/// non-deterministic and that is a defect.
#[test]
fn the_encoding_is_identical_on_every_platform() {
    /// FNV-1a. Inline to keep a hash dependency out of the crate for one test.
    fn digest(bytes: &[u8]) -> u64 {
        let mut hash = 0xcbf2_9ce4_8422_2325u64;
        for byte in bytes {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        hash
    }

    let grid = GridSpec {
        ni: 360,
        nj: 181,
        micro_degrees: 1_000_000,
    };
    let values: Vec<f32> = grid
        .points()
        .map(|(lon, lat)| ((lat / 90.0) * 25.0 + (lon / 180.0) * 5.0) as f32)
        .collect();

    let bytes = message(&spec(grid, Parameter::WindU, 12), &values).expect("builds");

    // The 31-byte local-use section records VectorEffects provenance.
    assert_eq!(bytes.len(), 130_530, "message length changed");
    // Pin the new section exactly, then retain the pre-provenance digest for
    // every other byte. This proves adding the memo did not alter field data.
    // Section 0 occupies 16 bytes and Section 1 occupies 21 bytes.
    assert_eq!(
        &bytes[37..68],
        b"\x00\x00\x00\x1f\x02Created with VectorEffects"
    );
    let mut without_provenance = bytes;
    without_provenance.drain(37..68);
    without_provenance[8..16].copy_from_slice(&130_499u64.to_be_bytes());
    assert_eq!(
        digest(&without_provenance),
        0x769b_cfb2_fb07_77c3,
        "encoding changed; re-record deliberately or find the non-determinism"
    );
}

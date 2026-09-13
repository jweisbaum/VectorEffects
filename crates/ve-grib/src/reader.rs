//! A minimal GRIB2 reader, for verification only.
//!
//! Compiled only with the `testing` feature, so it is never part of the shipped
//! binary (spec.md 12.5). It exists so every writer change is checked against an
//! independent parse rather than against itself, and so the round trip is
//! covered even where ecCodes is not installed.
//!
//! It understands exactly what the writer produces: sections 0-8 with templates
//! 3.0, 4.0 and 5.0. It is not a general GRIB2 decoder and should never grow
//! into one -- that is what ecCodes is for, in CI.

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::indexing_slicing,
    reason = "verification code: a panic on malformed input is the point"
)]

use std::collections::HashMap;

use crate::packing::{Packed, unpack};
use crate::writer::{ReferenceTime, from_i16_sm, from_i32_sm};

/// A decoded message.
#[derive(Debug, Clone, PartialEq)]
pub struct Decoded {
    pub discipline: u8,
    pub edition: u8,
    pub centre: u16,
    pub reference_time: ReferenceTime,
    pub category: u8,
    pub number: u8,
    pub forecast_hour: u32,
    pub surface_type: u8,
    pub surface_value: u32,
    pub ni: u32,
    pub nj: u32,
    pub la1: i32,
    pub lo1: i32,
    pub la2: i32,
    pub lo2: i32,
    pub di: u32,
    pub dj: u32,
    pub scanning_mode: u8,
    pub resolution_flags: u8,
    pub values: Vec<f32>,
    /// Bits per packed value, as section 5 recorded it.
    pub bits: u8,
}

fn u16_at(bytes: &[u8], at: usize) -> u16 {
    u16::from_be_bytes([bytes[at], bytes[at + 1]])
}

fn u32_at(bytes: &[u8], at: usize) -> u32 {
    u32::from_be_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

/// Parses one message, walking the section chain rather than assuming offsets.
pub fn decode(bytes: &[u8]) -> Decoded {
    assert_eq!(&bytes[0..4], b"GRIB", "magic");
    let discipline = bytes[6];
    let edition = bytes[7];
    let total = u64::from_be_bytes(bytes[8..16].try_into().unwrap());
    assert_eq!(total as usize, bytes.len(), "declared length");
    assert_eq!(&bytes[bytes.len() - 4..], b"7777", "end marker");

    let mut sections: HashMap<u8, &[u8]> = std::collections::HashMap::new();
    let mut at = 16;
    while at < bytes.len() - 4 {
        let length = u32_at(bytes, at) as usize;
        let number = bytes[at + 4];
        assert!(
            length >= 5 && at + length <= bytes.len(),
            "section {number} length"
        );
        sections.insert(number, &bytes[at..at + length]);
        at += length;
    }

    let s1 = sections[&1];
    let s3 = sections[&3];
    let s4 = sections[&4];
    let s5 = sections[&5];
    let s7 = sections[&7];

    // Section 5 gives the packing parameters; section 7 the bit stream.
    let count = u32_at(s5, 5) as usize;
    let packed = Packed {
        reference: f32::from_be_bytes(s5[11..15].try_into().unwrap()),
        binary_scale: from_i16_sm(u16_at(s5, 15)),
        decimal_scale: from_i16_sm(u16_at(s5, 17)),
        bits: s5[19],
        data: s7[5..].to_vec(),
        count,
    };
    let values = unpack(&packed);
    let s6 = sections[&6];
    let values = if s6[5] == 0 {
        let mut present = values.into_iter();
        let count = (u32_at(s3, 30) * u32_at(s3, 34)) as usize;
        let expanded = (0..count)
            .map(|at| {
                if s6[6 + at / 8] & (1 << (7 - at % 8)) != 0 {
                    present.next().expect("bitmap value")
                } else {
                    f32::NAN
                }
            })
            .collect();
        assert!(present.next().is_none(), "all packed values used");
        expanded
    } else {
        assert_eq!(s6[5], 255, "supported bitmap");
        values
    };

    Decoded {
        bits: packed.bits,
        discipline,
        edition,
        centre: u16_at(s1, 5),
        reference_time: ReferenceTime {
            year: u16_at(s1, 12),
            month: s1[14],
            day: s1[15],
            hour: s1[16],
            minute: s1[17],
            second: s1[18],
        },
        category: s4[9],
        number: s4[10],
        forecast_hour: u32_at(s4, 18),
        surface_type: s4[22],
        surface_value: u32_at(s4, 24),
        ni: u32_at(s3, 30),
        nj: u32_at(s3, 34),
        la1: from_i32_sm(u32_at(s3, 46)),
        lo1: from_i32_sm(u32_at(s3, 50)),
        resolution_flags: s3[54],
        la2: from_i32_sm(u32_at(s3, 55)),
        lo2: from_i32_sm(u32_at(s3, 59)),
        di: u32_at(s3, 63),
        dj: u32_at(s3, 67),
        scanning_mode: s3[71],
        values,
    }
}
